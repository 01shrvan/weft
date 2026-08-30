use weft::error::{ErrorCode, H2Error};
use weft::frame::{flag, Frame, FrameDecoder, FrameHeader, FrameType, FRAME_HEADER_LEN};

fn drain(dec: &mut FrameDecoder) -> Vec<Frame> {
    let mut out = Vec::new();
    while let Some(f) = dec.next_frame().expect("decode must not fail") {
        out.push(f);
    }
    out
}

fn session() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&Frame::new(FrameType::Settings, 0, 0, vec![0, 3, 0, 0, 0, 100]).encode());
    bytes.extend_from_slice(&Frame::new(FrameType::Settings, flag::ACK, 0, Vec::new()).encode());
    bytes.extend_from_slice(&Frame::new(FrameType::Ping, 0, 0, vec![1, 2, 3, 4, 5, 6, 7, 8]).encode());
    bytes.extend_from_slice(&Frame::new(FrameType::Data, flag::END_STREAM, 1, b"hello".to_vec()).encode());
    bytes.extend_from_slice(&Frame::new(FrameType::GoAway, 0, 0, vec![0, 0, 0, 1, 0, 0, 0, 0]).encode());
    bytes
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn settings_ack_matches_the_rfc_byte_for_byte() {
    let f = Frame::new(FrameType::Settings, flag::ACK, 0, Vec::new());
    assert_eq!(f.encode(), vec![0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00]);
}

#[test]
fn header_round_trips_through_bytes() {
    let h = FrameHeader {
        length: 5,
        kind: FrameType::Data,
        flags: flag::END_STREAM,
        stream_id: 7,
    };
    let mut out = Vec::new();
    h.write_into(&mut out);
    assert_eq!(out.len(), FRAME_HEADER_LEN);
    assert_eq!(FrameHeader::parse(&out), h);
}

#[test]
fn reserved_bit_is_stripped_from_the_stream_id() {
    let raw = [0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff];
    assert_eq!(FrameHeader::parse(&raw).stream_id, 0x7fff_ffff);
}

#[test]
fn unknown_frame_types_survive_a_round_trip() {
    assert_eq!(FrameType::from_u8(0x63), FrameType::Unknown(0x63));
    assert_eq!(FrameType::Unknown(0x63).as_u8(), 0x63);
}

#[test]
fn several_frames_arriving_in_one_read_all_decode() {
    let mut dec = FrameDecoder::new();
    dec.feed(&session());
    let frames = drain(&mut dec);
    assert_eq!(frames.len(), 5);
    assert_eq!(frames[2].header.kind, FrameType::Ping);
    assert_eq!(frames[3].payload, b"hello");
    assert!(frames[3].header.has(flag::END_STREAM));
}

#[test]
fn a_frame_split_across_reads_waits_for_the_rest() {
    let bytes = Frame::new(FrameType::Ping, 0, 0, vec![9; 8]).encode();
    let mut dec = FrameDecoder::new();
    dec.feed(&bytes[..4]);
    assert!(dec.next_frame().unwrap().is_none());
    dec.feed(&bytes[4..12]);
    assert!(dec.next_frame().unwrap().is_none());
    dec.feed(&bytes[12..]);
    let f = dec.next_frame().unwrap().expect("frame completes");
    assert_eq!(f.payload, vec![9; 8]);
}

#[test]
fn chunking_never_changes_the_decoded_result() {
    let bytes = session();
    let expected = {
        let mut dec = FrameDecoder::new();
        dec.feed(&bytes);
        drain(&mut dec)
    };

    let mut one_byte_at_a_time = FrameDecoder::new();
    let mut got = Vec::new();
    for b in &bytes {
        one_byte_at_a_time.feed(&[*b]);
        while let Some(f) = one_byte_at_a_time.next_frame().unwrap() {
            got.push(f);
        }
    }
    assert_eq!(got, expected, "byte-at-a-time decode diverged");

    let mut rng = Rng(0x9e3779b97f4a7c15);
    for _ in 0..200 {
        let mut dec = FrameDecoder::new();
        let mut got = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let take = 1 + (rng.next() % 13) as usize;
            let end = usize::min(i + take, bytes.len());
            dec.feed(&bytes[i..end]);
            while let Some(f) = dec.next_frame().unwrap() {
                got.push(f);
            }
            i = end;
        }
        assert_eq!(got, expected, "random chunking diverged");
    }
}

#[test]
fn a_frame_larger_than_the_negotiated_max_is_a_connection_error() {
    let mut dec = FrameDecoder::new();
    dec.set_max_frame_size(16);
    dec.feed(&Frame::new(FrameType::Data, 0, 1, vec![0; 32]).encode());
    assert_eq!(
        dec.next_frame().unwrap_err(),
        H2Error::Connection(ErrorCode::FrameSizeError)
    );
}

#[test]
fn oversize_is_rejected_before_the_payload_arrives() {
    let mut dec = FrameDecoder::new();
    dec.set_max_frame_size(16);
    let bytes = Frame::new(FrameType::Data, 0, 1, vec![0; 32]).encode();
    dec.feed(&bytes[..FRAME_HEADER_LEN]);
    assert!(dec.next_frame().is_err(), "must not wait for 32 bytes it will never accept");
}

#[test]
fn error_codes_round_trip_and_unknown_becomes_internal() {
    for c in [
        ErrorCode::NoError,
        ErrorCode::ProtocolError,
        ErrorCode::FlowControlError,
        ErrorCode::FrameSizeError,
        ErrorCode::CompressionError,
        ErrorCode::Http11Required,
    ] {
        assert_eq!(ErrorCode::from_u32(c.as_u32()), c);
    }
    assert_eq!(ErrorCode::from_u32(0xffff), ErrorCode::InternalError);
}

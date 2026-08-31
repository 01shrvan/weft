use weft::error::{ErrorCode, H2Error};
use weft::hpack::huffman;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[test]
fn rfc7541_c41_www_example_com() {
    let got = huffman::encode(b"www.example.com");
    assert_eq!(hex(&got), "f1e3c2e5f23a6ba0ab90f4ff");
    assert_eq!(huffman::decode(&got).unwrap(), b"www.example.com");
}

#[test]
fn rfc7541_c42_no_cache() {
    let got = huffman::encode(b"no-cache");
    assert_eq!(hex(&got), "a8eb10649cbf");
    assert_eq!(huffman::decode(&got).unwrap(), b"no-cache");
}

#[test]
fn rfc7541_c43_custom_key_and_value() {
    let key = huffman::encode(b"custom-key");
    assert_eq!(hex(&key), "25a849e95ba97d7f");
    let value = huffman::encode(b"custom-value");
    assert_eq!(hex(&value), "25a849e95bb8e8b4bf");
    assert_eq!(huffman::decode(&key).unwrap(), b"custom-key");
    assert_eq!(huffman::decode(&value).unwrap(), b"custom-value");
}

#[test]
fn every_byte_value_round_trips_alone_and_in_a_run() {
    let all: Vec<u8> = (0u16..=255).map(|b| b as u8).collect();
    assert_eq!(huffman::decode(&huffman::encode(&all)).unwrap(), all);
    for b in 0u16..=255 {
        let one = [b as u8];
        let round = huffman::decode(&huffman::encode(&one)).unwrap();
        assert_eq!(round, one, "byte {b} did not survive");
    }
}

#[test]
fn encoded_len_agrees_with_encode() {
    for sample in [
        &b""[..],
        &b"a"[..],
        &b"www.example.com"[..],
        &b"custom-value"[..],
        &b"\x00\xff\x80\x7f"[..],
    ] {
        assert_eq!(huffman::encoded_len(sample), huffman::encode(sample).len());
    }
}

#[test]
fn an_encoded_eos_symbol_is_a_compression_error() {
    let thirty_ones = [0xff, 0xff, 0xff, 0xff];
    assert_eq!(
        huffman::decode(&thirty_ones).unwrap_err(),
        H2Error::Connection(ErrorCode::CompressionError)
    );
}

#[test]
fn padding_that_is_not_all_ones_is_a_compression_error() {
    assert_eq!(
        huffman::decode(&[0x00]).unwrap_err(),
        H2Error::Connection(ErrorCode::CompressionError)
    );
}

#[test]
fn padding_longer_than_seven_bits_is_a_compression_error() {
    let mut padded = huffman::encode(b"a");
    padded.push(0xff);
    assert!(huffman::decode(&padded).is_err(), "a whole byte of padding must be rejected");
}

#[test]
fn the_empty_string_encodes_and_decodes_to_nothing() {
    assert_eq!(huffman::encode(b""), Vec::<u8>::new());
    assert_eq!(huffman::decode(&[]).unwrap(), Vec::<u8>::new());
}

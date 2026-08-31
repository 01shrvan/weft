use std::collections::HashMap;
use std::io::{self, Read, Write};

use crate::error::{ErrorCode, H2Error, H2Result};
use crate::frame::{flag, Frame, FrameDecoder, FrameType, PREFACE};
use crate::hpack::decoder::Decoder;
use crate::hpack::encoder;
use crate::hpack::table::Header;
use crate::settings::Settings;
use crate::stream::{validate, State, Stream};

const READ_CHUNK: usize = 8 * 1024;
const BODY: &[u8] = b"weft\n";
const STATUS_200_INDEX: u32 = 8;
const CONTENT_LENGTH_INDEX: u32 = 28;
const CONTENT_TYPE_INDEX: u32 = 31;

struct Assembly {
    stream_id: u32,
    block: Vec<u8>,
    end_stream: bool,
}

pub struct Connection<S> {
    io: S,
    dec: FrameDecoder,
    hpack: Decoder,
    streams: HashMap<u32, Stream>,
    assembly: Option<Assembly>,
    pub local: Settings,
    pub remote: Settings,
    highest_client_stream: u32,
    recv_window: i64,
    send_window: i64,
    saw_first_settings: bool,
    goaway_sent: bool,
}

impl<S: Read + Write> Connection<S> {
    pub fn new(io: S) -> Self {
        let mut local = Settings::default();
        local.max_concurrent_streams = Some(100);
        Connection {
            io,
            dec: FrameDecoder::new(),
            hpack: Decoder::new(local.header_table_size as usize),
            streams: HashMap::new(),
            assembly: None,
            remote: Settings::default(),
            highest_client_stream: 0,
            recv_window: i64::from(local.initial_window_size),
            send_window: 65_535,
            saw_first_settings: false,
            goaway_sent: false,
            local,
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        if let Err(e) = self.handshake() {
            return self.fail(e);
        }
        let mut chunk = [0u8; READ_CHUNK];
        loop {
            let n = match self.io.read(&mut chunk) {
                Ok(n) => n,
                Err(_) => return Ok(()),
            };
            if n == 0 {
                return Ok(());
            }
            self.dec.feed(&chunk[..n]);
            loop {
                match self.dec.next_frame() {
                    Ok(Some(f)) => {
                        if let Err(e) = self.dispatch(f) {
                            return self.fail(e);
                        }
                    }
                    Ok(None) => break,
                    Err(e) => return self.fail(e),
                }
            }
        }
    }

    fn handshake(&mut self) -> H2Result<()> {
        let mut seen = 0usize;
        let mut byte = [0u8; 1];
        while seen < PREFACE.len() {
            match self.io.read(&mut byte) {
                Ok(0) => return Err(H2Error::Connection(ErrorCode::ProtocolError)),
                Ok(_) => {
                    if byte[0] != PREFACE[seen] {
                        return Err(H2Error::Connection(ErrorCode::ProtocolError));
                    }
                    seen += 1;
                }
                Err(_) => return Err(H2Error::Connection(ErrorCode::ProtocolError)),
            }
        }
        let settings = Frame::new(FrameType::Settings, 0, 0, self.local.encode_payload());
        self.write_frame(&settings)
    }

    fn write_frame(&mut self, f: &Frame) -> H2Result<()> {
        self.io
            .write_all(&f.encode())
            .map_err(|_| H2Error::Connection(ErrorCode::InternalError))?;
        self.io
            .flush()
            .map_err(|_| H2Error::Connection(ErrorCode::InternalError))
    }

    fn conn_err<T>(code: ErrorCode) -> H2Result<T> {
        Err(H2Error::Connection(code))
    }

    fn strip_padding(payload: &[u8], padded: bool) -> H2Result<&[u8]> {
        if !padded {
            return Ok(payload);
        }
        if payload.is_empty() {
            return Self::conn_err(ErrorCode::ProtocolError);
        }
        let pad = payload[0] as usize;
        if pad >= payload.len() {
            return Self::conn_err(ErrorCode::ProtocolError);
        }
        Ok(&payload[1..payload.len() - pad])
    }

    fn dispatch(&mut self, f: Frame) -> H2Result<()> {
        let h = f.header;

        if self.assembly.is_some() && h.kind != FrameType::Continuation {
            return Self::conn_err(ErrorCode::ProtocolError);
        }
        if !self.saw_first_settings && h.kind != FrameType::Settings {
            return Self::conn_err(ErrorCode::ProtocolError);
        }

        match h.kind {
            FrameType::Settings => {
                if h.stream_id != 0 {
                    return Self::conn_err(ErrorCode::ProtocolError);
                }
                self.saw_first_settings = true;
                if h.has(flag::ACK) {
                    if h.length != 0 {
                        return Self::conn_err(ErrorCode::FrameSizeError);
                    }
                    return Ok(());
                }
                let mut next = self.remote;
                next.apply(&f.payload)?;
                self.remote = next;
                self.hpack.set_settings_limit(next.header_table_size as usize);
                let ack = Frame::new(FrameType::Settings, flag::ACK, 0, Vec::new());
                self.write_frame(&ack)
            }

            FrameType::Ping => {
                if h.stream_id != 0 {
                    return Self::conn_err(ErrorCode::ProtocolError);
                }
                if h.length != 8 {
                    return Self::conn_err(ErrorCode::FrameSizeError);
                }
                if h.has(flag::ACK) {
                    return Ok(());
                }
                let pong = Frame::new(FrameType::Ping, flag::ACK, 0, f.payload);
                self.write_frame(&pong)
            }

            FrameType::GoAway => {
                if h.stream_id != 0 {
                    return Self::conn_err(ErrorCode::ProtocolError);
                }
                if h.length < 8 {
                    return Self::conn_err(ErrorCode::FrameSizeError);
                }
                Self::conn_err(ErrorCode::NoError)
            }

            FrameType::WindowUpdate => {
                if h.length != 4 {
                    return Self::conn_err(ErrorCode::FrameSizeError);
                }
                let raw = [f.payload[0], f.payload[1], f.payload[2], f.payload[3]];
                let inc = i64::from(u32::from_be_bytes(raw) & 0x7fff_ffff);
                if inc == 0 {
                    if h.stream_id == 0 {
                        return Self::conn_err(ErrorCode::ProtocolError);
                    }
                    return Err(H2Error::Stream {
                        id: h.stream_id,
                        code: ErrorCode::ProtocolError,
                    });
                }
                if h.stream_id == 0 {
                    self.send_window += inc;
                    if self.send_window > 0x7fff_ffff {
                        return Self::conn_err(ErrorCode::FlowControlError);
                    }
                    return Ok(());
                }
                if let Some(s) = self.streams.get_mut(&h.stream_id) {
                    s.send_window += inc;
                    if s.send_window > 0x7fff_ffff {
                        return Err(H2Error::Stream {
                            id: h.stream_id,
                            code: ErrorCode::FlowControlError,
                        });
                    }
                }
                Ok(())
            }

            FrameType::Priority => {
                if h.stream_id == 0 {
                    return Self::conn_err(ErrorCode::ProtocolError);
                }
                if h.length != 5 {
                    return Err(H2Error::Stream {
                        id: h.stream_id,
                        code: ErrorCode::FrameSizeError,
                    });
                }
                Ok(())
            }

            FrameType::RstStream => {
                if h.stream_id == 0 {
                    return Self::conn_err(ErrorCode::ProtocolError);
                }
                if h.length != 4 {
                    return Self::conn_err(ErrorCode::FrameSizeError);
                }
                if h.stream_id > self.highest_client_stream {
                    return Self::conn_err(ErrorCode::ProtocolError);
                }
                if let Some(s) = self.streams.get_mut(&h.stream_id) {
                    s.state = State::Closed;
                }
                Ok(())
            }

            FrameType::PushPromise => Self::conn_err(ErrorCode::ProtocolError),

            FrameType::Headers => {
                if h.stream_id == 0 || h.stream_id % 2 == 0 {
                    return Self::conn_err(ErrorCode::ProtocolError);
                }
                if h.stream_id < self.highest_client_stream {
                    return Self::conn_err(ErrorCode::ProtocolError);
                }
                if let Some(s) = self.streams.get(&h.stream_id) {
                    if s.state != State::Idle {
                        return Self::conn_err(ErrorCode::StreamClosed);
                    }
                }
                self.highest_client_stream = h.stream_id;

                let body = Self::strip_padding(&f.payload, h.has(flag::PADDED))?;
                let block = if h.has(flag::PRIORITY) {
                    if body.len() < 5 {
                        return Self::conn_err(ErrorCode::FrameSizeError);
                    }
                    let dep = u32::from_be_bytes([body[0], body[1], body[2], body[3]]) & 0x7fff_ffff;
                    if dep == h.stream_id {
                        return Self::conn_err(ErrorCode::ProtocolError);
                    }
                    &body[5..]
                } else {
                    body
                };

                let mut stream = Stream::new(
                    h.stream_id,
                    i64::from(self.remote.initial_window_size),
                    i64::from(self.local.initial_window_size),
                );
                stream.state = State::Open;
                let assembly = Assembly {
                    stream_id: h.stream_id,
                    block: block.to_vec(),
                    end_stream: h.has(flag::END_STREAM),
                };
                self.streams.insert(h.stream_id, stream);

                if h.has(flag::END_HEADERS) {
                    self.finish_headers(assembly)
                } else {
                    self.assembly = Some(assembly);
                    Ok(())
                }
            }

            FrameType::Continuation => {
                let mut assembly = match self.assembly.take() {
                    Some(a) => a,
                    None => return Self::conn_err(ErrorCode::ProtocolError),
                };
                if assembly.stream_id != h.stream_id {
                    return Self::conn_err(ErrorCode::ProtocolError);
                }
                assembly.block.extend_from_slice(&f.payload);
                if h.has(flag::END_HEADERS) {
                    self.finish_headers(assembly)
                } else {
                    self.assembly = Some(assembly);
                    Ok(())
                }
            }

            FrameType::Data => {
                if h.stream_id == 0 {
                    return Self::conn_err(ErrorCode::ProtocolError);
                }
                let len = i64::from(h.length);
                if len > self.recv_window {
                    return Self::conn_err(ErrorCode::FlowControlError);
                }
                let state = self.streams.get(&h.stream_id).map(|s| s.state);
                match state {
                    None => return Self::conn_err(ErrorCode::ProtocolError),
                    Some(State::Idle) => return Self::conn_err(ErrorCode::ProtocolError),
                    Some(State::Closed) => return Self::conn_err(ErrorCode::StreamClosed),
                    Some(State::HalfClosedRemote) => {
                        return Err(H2Error::Stream {
                            id: h.stream_id,
                            code: ErrorCode::StreamClosed,
                        })
                    }
                    _ => {}
                }
                Self::strip_padding(&f.payload, h.has(flag::PADDED))?;
                if h.length > 0 {
                    let refill = Frame::new(
                        FrameType::WindowUpdate,
                        0,
                        0,
                        (h.length & 0x7fff_ffff).to_be_bytes().to_vec(),
                    );
                    self.write_frame(&refill)?;
                }
                if h.has(flag::END_STREAM) {
                    if let Some(s) = self.streams.get_mut(&h.stream_id) {
                        s.state = State::HalfClosedRemote;
                    }
                    self.respond(h.stream_id)?;
                }
                Ok(())
            }

            FrameType::Unknown(_) => Ok(()),
        }
    }

    fn finish_headers(&mut self, assembly: Assembly) -> H2Result<()> {
        let headers: Vec<Header> = self.hpack.decode(&assembly.block)?;
        validate(assembly.stream_id, &headers)?;
        if let Some(s) = self.streams.get_mut(&assembly.stream_id) {
            s.state = if assembly.end_stream {
                State::HalfClosedRemote
            } else {
                State::Open
            };
        }
        if assembly.end_stream {
            self.respond(assembly.stream_id)?;
        }
        Ok(())
    }

    fn respond(&mut self, stream_id: u32) -> H2Result<()> {
        let mut block = Vec::new();
        encoder::indexed(&mut block, STATUS_200_INDEX);
        encoder::literal_indexed_name(&mut block, CONTENT_TYPE_INDEX, b"text/plain");
        let len = BODY.len().to_string();
        encoder::literal_indexed_name(&mut block, CONTENT_LENGTH_INDEX, len.as_bytes());

        let headers = Frame::new(FrameType::Headers, flag::END_HEADERS, stream_id, block);
        self.write_frame(&headers)?;
        let data = Frame::new(FrameType::Data, flag::END_STREAM, stream_id, BODY.to_vec());
        self.write_frame(&data)?;
        if let Some(s) = self.streams.get_mut(&stream_id) {
            s.state = State::Closed;
        }
        Ok(())
    }

    fn goaway(&mut self, code: ErrorCode) -> io::Result<()> {
        if self.goaway_sent {
            return Ok(());
        }
        self.goaway_sent = true;
        let mut payload = Vec::with_capacity(8);
        payload.extend_from_slice(&(self.highest_client_stream & 0x7fff_ffff).to_be_bytes());
        payload.extend_from_slice(&code.as_u32().to_be_bytes());
        let f = Frame::new(FrameType::GoAway, 0, 0, payload);
        self.io.write_all(&f.encode())?;
        self.io.flush()
    }

    fn fail(&mut self, e: H2Error) -> io::Result<()> {
        match e {
            H2Error::Connection(code) => self.goaway(code),
            H2Error::Stream { id, code } => {
                let body = code.as_u32().to_be_bytes().to_vec();
                let f = Frame::new(FrameType::RstStream, 0, id, body);
                self.io.write_all(&f.encode())?;
                self.io.flush()
            }
        }
    }
}

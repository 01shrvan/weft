use std::collections::HashMap;
use std::io::{self, Read, Write};

use crate::error::{ErrorCode, H2Error, H2Result};
use crate::frame::{flag, Frame, FrameDecoder, FrameType, PREFACE};
use crate::hpack::decoder::Decoder;
use crate::hpack::encoder;
use crate::hpack::table::Header;
use crate::settings::Settings;
use crate::stream::{validate, validate_trailers, State, Stream};

const READ_CHUNK: usize = 8 * 1024;
const BODY: &[u8] = b"weft\n";
const STATUS_200_INDEX: u32 = 8;
const CONTENT_LENGTH_INDEX: u32 = 28;
const CONTENT_TYPE_INDEX: u32 = 31;
const MAX_CONCURRENT: usize = 100;
const WINDOW_CEILING: i64 = 0x7fff_ffff;

struct Assembly {
    stream_id: u32,
    block: Vec<u8>,
    end_stream: bool,
    trailers: bool,
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
    out: Vec<u8>,
    saw_first_settings: bool,
    goaway_sent: bool,
}

impl<S: Read + Write> Connection<S> {
    pub fn new(io: S) -> Self {
        let local = Settings {
            max_concurrent_streams: Some(MAX_CONCURRENT as u32),
            ..Default::default()
        };
        let remote = Settings::default();
        Connection {
            io,
            dec: FrameDecoder::new(),
            hpack: Decoder::new(local.header_table_size as usize),
            streams: HashMap::new(),
            assembly: None,
            highest_client_stream: 0,
            recv_window: i64::from(local.initial_window_size),
            send_window: i64::from(remote.initial_window_size),
            out: Vec::with_capacity(16 * 1024),
            saw_first_settings: false,
            goaway_sent: false,
            remote,
            local,
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        if let Err(e) = self.handshake() {
            return self.fail(e);
        }
        let mut chunk = [0u8; READ_CHUNK];
        loop {
            self.flush_out()?;
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
                        if let Err(e) = self.flush_sends() {
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
        f.header.write_into(&mut self.out);
        self.out.extend_from_slice(&f.payload);
        Ok(())
    }

    fn flush_out(&mut self) -> io::Result<()> {
        if self.out.is_empty() {
            return Ok(());
        }
        self.io.write_all(&self.out)?;
        self.io.flush()?;
        self.out.clear();
        Ok(())
    }

    fn conn_err<T>(code: ErrorCode) -> H2Result<T> {
        Err(H2Error::Connection(code))
    }

    fn stream_err<T>(id: u32, code: ErrorCode) -> H2Result<T> {
        Err(H2Error::Stream { id, code })
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

    fn active_streams(&self) -> usize {
        self.streams.values().filter(|s| s.active()).count()
    }

    fn flush_sends(&mut self) -> H2Result<()> {
        let ids: Vec<u32> = self
            .streams
            .values()
            .filter(|s| !s.finished && !s.pending.is_empty())
            .map(|s| s.id)
            .collect();
        for id in ids {
            self.flush_stream(id)?;
        }
        Ok(())
    }

    fn flush_stream(&mut self, id: u32) -> H2Result<()> {
        loop {
            let max_frame = i64::from(self.remote.max_frame_size);
            let (window, remaining) = match self.streams.get(&id) {
                Some(s) => (i64::min(s.send_window, self.send_window), s.pending.len() as i64),
                None => return Ok(()),
            };
            if remaining == 0 {
                return Ok(());
            }
            let take = i64::min(i64::min(window, remaining), max_frame);
            if take <= 0 {
                return Ok(());
            }
            let take = take as usize;
            let (chunk, last) = match self.streams.get_mut(&id) {
                Some(s) => {
                    let chunk: Vec<u8> = s.pending.drain(..take).collect();
                    let last = s.pending.is_empty();
                    s.send_window -= take as i64;
                    (chunk, last)
                }
                None => return Ok(()),
            };
            self.send_window -= take as i64;
            let flags = if last { flag::END_STREAM } else { 0 };
            let data = Frame::new(FrameType::Data, flags, id, chunk);
            self.write_frame(&data)?;
            if last {
                if let Some(s) = self.streams.get_mut(&id) {
                    s.finished = true;
                    s.state = State::Closed;
                }
                return Ok(());
            }
        }
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
                let previous = self.remote.initial_window_size;
                let mut next = self.remote;
                next.apply(&f.payload)?;
                let delta = i64::from(next.initial_window_size) - i64::from(previous);
                if delta != 0 {
                    for s in self.streams.values_mut() {
                        s.send_window += delta;
                        if s.send_window > WINDOW_CEILING {
                            return Self::conn_err(ErrorCode::FlowControlError);
                        }
                    }
                }
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
                if h.stream_id == 0 {
                    if inc == 0 {
                        return Self::conn_err(ErrorCode::ProtocolError);
                    }
                    self.send_window += inc;
                    if self.send_window > WINDOW_CEILING {
                        return Self::conn_err(ErrorCode::FlowControlError);
                    }
                    return Ok(());
                }
                if h.stream_id > self.highest_client_stream {
                    return Self::conn_err(ErrorCode::ProtocolError);
                }
                if inc == 0 {
                    return Self::stream_err(h.stream_id, ErrorCode::ProtocolError);
                }
                if let Some(s) = self.streams.get_mut(&h.stream_id) {
                    s.send_window += inc;
                    if s.send_window > WINDOW_CEILING {
                        return Self::stream_err(h.stream_id, ErrorCode::FlowControlError);
                    }
                }
                Ok(())
            }

            FrameType::Priority => {
                if h.stream_id == 0 {
                    return Self::conn_err(ErrorCode::ProtocolError);
                }
                if h.length != 5 {
                    return Self::stream_err(h.stream_id, ErrorCode::FrameSizeError);
                }
                let raw = [f.payload[0], f.payload[1], f.payload[2], f.payload[3]];
                let dep = u32::from_be_bytes(raw) & 0x7fff_ffff;
                if dep == h.stream_id {
                    return Self::stream_err(h.stream_id, ErrorCode::ProtocolError);
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
                    s.finished = true;
                    s.pending.clear();
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

                let existing = self.streams.get(&h.stream_id).map(|s| s.state);
                let trailers = match existing {
                    Some(State::Open) => {
                        if !h.has(flag::END_STREAM) {
                            return Self::conn_err(ErrorCode::ProtocolError);
                        }
                        true
                    }
                    Some(_) => return Self::conn_err(ErrorCode::StreamClosed),
                    None => false,
                };

                if !trailers && self.active_streams() >= MAX_CONCURRENT {
                    return Self::stream_err(h.stream_id, ErrorCode::RefusedStream);
                }

                self.highest_client_stream = h.stream_id;

                let body = Self::strip_padding(&f.payload, h.has(flag::PADDED))?;
                let block = if h.has(flag::PRIORITY) {
                    if body.len() < 5 {
                        return Self::conn_err(ErrorCode::FrameSizeError);
                    }
                    let raw = [body[0], body[1], body[2], body[3]];
                    if u32::from_be_bytes(raw) & 0x7fff_ffff == h.stream_id {
                        return Self::conn_err(ErrorCode::ProtocolError);
                    }
                    &body[5..]
                } else {
                    body
                };

                if !trailers {
                    let initial = i64::from(self.remote.initial_window_size);
                    let mut stream = Stream::new(h.stream_id, initial);
                    stream.state = State::Open;
                    self.streams.insert(h.stream_id, stream);
                }

                let assembly = Assembly {
                    stream_id: h.stream_id,
                    block: block.to_vec(),
                    end_stream: h.has(flag::END_STREAM),
                    trailers,
                };
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
                match self.streams.get(&h.stream_id).map(|s| s.state) {
                    None => return Self::conn_err(ErrorCode::ProtocolError),
                    Some(State::Idle) => return Self::conn_err(ErrorCode::ProtocolError),
                    Some(State::Closed) => return Self::conn_err(ErrorCode::StreamClosed),
                    Some(State::HalfClosedRemote) => {
                        return Self::stream_err(h.stream_id, ErrorCode::StreamClosed)
                    }
                    _ => {}
                }
                let body = Self::strip_padding(&f.payload, h.has(flag::PADDED))?;
                let received = body.len() as u64;
                if let Some(s) = self.streams.get_mut(&h.stream_id) {
                    s.data_received += received;
                }
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
                    self.check_content_length(h.stream_id)?;
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

    fn check_content_length(&self, id: u32) -> H2Result<()> {
        if let Some(s) = self.streams.get(&id) {
            if let Some(declared) = s.content_length {
                if declared != s.data_received {
                    return Self::stream_err(id, ErrorCode::ProtocolError);
                }
            }
        }
        Ok(())
    }

    fn finish_headers(&mut self, assembly: Assembly) -> H2Result<()> {
        let headers: Vec<Header> = self.hpack.decode(&assembly.block)?;
        if assembly.trailers {
            validate_trailers(assembly.stream_id, &headers)?;
        } else {
            let req = validate(assembly.stream_id, &headers)?;
            if let Some(s) = self.streams.get_mut(&assembly.stream_id) {
                s.content_length = req.content_length;
            }
        }
        if assembly.end_stream {
            self.check_content_length(assembly.stream_id)?;
            if let Some(s) = self.streams.get_mut(&assembly.stream_id) {
                s.state = State::HalfClosedRemote;
            }
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
        if let Some(s) = self.streams.get_mut(&stream_id) {
            s.headers_sent = true;
            s.pending = BODY.to_vec();
            s.state = State::HalfClosedLocal;
        }
        self.flush_stream(stream_id)
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
        f.header.write_into(&mut self.out);
        self.out.extend_from_slice(&f.payload);
        self.flush_out()
    }

    fn fail(&mut self, e: H2Error) -> io::Result<()> {
        match e {
            H2Error::Connection(code) => self.goaway(code),
            H2Error::Stream { id, code } => {
                let body = code.as_u32().to_be_bytes().to_vec();
                let f = Frame::new(FrameType::RstStream, 0, id, body);
                f.header.write_into(&mut self.out);
                self.out.extend_from_slice(&f.payload);
                self.flush_out()
            }
        }
    }
}

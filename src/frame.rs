use crate::error::{ErrorCode, H2Error, H2Result};

pub const FRAME_HEADER_LEN: usize = 9;
pub const DEFAULT_MAX_FRAME_SIZE: u32 = 16_384;
pub const MAX_ALLOWED_FRAME_SIZE: u32 = 16_777_215;
pub const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub mod flag {
    pub const ACK: u8 = 0x1;
    pub const END_STREAM: u8 = 0x1;
    pub const END_HEADERS: u8 = 0x4;
    pub const PADDED: u8 = 0x8;
    pub const PRIORITY: u8 = 0x20;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    Data,
    Headers,
    Priority,
    RstStream,
    Settings,
    PushPromise,
    Ping,
    GoAway,
    WindowUpdate,
    Continuation,
    Unknown(u8),
}

impl FrameType {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x0 => FrameType::Data,
            0x1 => FrameType::Headers,
            0x2 => FrameType::Priority,
            0x3 => FrameType::RstStream,
            0x4 => FrameType::Settings,
            0x5 => FrameType::PushPromise,
            0x6 => FrameType::Ping,
            0x7 => FrameType::GoAway,
            0x8 => FrameType::WindowUpdate,
            0x9 => FrameType::Continuation,
            other => FrameType::Unknown(other),
        }
    }

    pub fn as_u8(self) -> u8 {
        match self {
            FrameType::Data => 0x0,
            FrameType::Headers => 0x1,
            FrameType::Priority => 0x2,
            FrameType::RstStream => 0x3,
            FrameType::Settings => 0x4,
            FrameType::PushPromise => 0x5,
            FrameType::Ping => 0x6,
            FrameType::GoAway => 0x7,
            FrameType::WindowUpdate => 0x8,
            FrameType::Continuation => 0x9,
            FrameType::Unknown(v) => v,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub length: u32,
    pub kind: FrameType,
    pub flags: u8,
    pub stream_id: u32,
}

impl FrameHeader {
    pub fn parse(b: &[u8]) -> Self {
        debug_assert!(b.len() >= FRAME_HEADER_LEN);
        let length = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        let raw_id = u32::from_be_bytes([b[5], b[6], b[7], b[8]]);
        FrameHeader {
            length,
            kind: FrameType::from_u8(b[3]),
            flags: b[4],
            stream_id: raw_id & 0x7fff_ffff,
        }
    }

    pub fn write_into(&self, out: &mut Vec<u8>) {
        out.push((self.length >> 16) as u8);
        out.push((self.length >> 8) as u8);
        out.push(self.length as u8);
        out.push(self.kind.as_u8());
        out.push(self.flags);
        out.extend_from_slice(&(self.stream_id & 0x7fff_ffff).to_be_bytes());
    }

    pub fn has(&self, f: u8) -> bool {
        self.flags & f != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub header: FrameHeader,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(kind: FrameType, flags: u8, stream_id: u32, payload: Vec<u8>) -> Self {
        Frame {
            header: FrameHeader {
                length: payload.len() as u32,
                kind,
                flags,
                stream_id,
            },
            payload,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(FRAME_HEADER_LEN + self.payload.len());
        self.header.write_into(&mut out);
        out.extend_from_slice(&self.payload);
        out
    }
}

const COMPACT_THRESHOLD: usize = 32 * 1024;

pub struct FrameDecoder {
    buf: Vec<u8>,
    pos: usize,
    max_frame_size: u32,
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameDecoder {
    pub fn new() -> Self {
        FrameDecoder {
            buf: Vec::new(),
            pos: 0,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
        }
    }

    pub fn set_max_frame_size(&mut self, n: u32) {
        self.max_frame_size = n;
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn buffered(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn compact(&mut self) {
        if self.pos == 0 {
            return;
        }
        if self.pos >= self.buf.len() {
            self.buf.clear();
            self.pos = 0;
        } else if self.pos >= COMPACT_THRESHOLD {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
    }

    pub fn next_frame(&mut self) -> H2Result<Option<Frame>> {
        if self.buffered() < FRAME_HEADER_LEN {
            self.compact();
            return Ok(None);
        }
        let header = FrameHeader::parse(&self.buf[self.pos..self.pos + FRAME_HEADER_LEN]);
        if header.length > self.max_frame_size {
            return Err(H2Error::Connection(ErrorCode::FrameSizeError));
        }
        let total = FRAME_HEADER_LEN + header.length as usize;
        if self.buffered() < total {
            self.compact();
            return Ok(None);
        }
        let start = self.pos + FRAME_HEADER_LEN;
        let payload = self.buf[start..start + header.length as usize].to_vec();
        self.pos += total;
        self.compact();
        Ok(Some(Frame { header, payload }))
    }
}

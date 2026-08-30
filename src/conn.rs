use std::io::{self, Read, Write};

use crate::error::{ErrorCode, H2Error, H2Result};
use crate::frame::{flag, Frame, FrameDecoder, FrameType, PREFACE};
use crate::settings::Settings;

const READ_CHUNK: usize = 8 * 1024;

pub struct Connection<S> {
    io: S,
    dec: FrameDecoder,
    pub local: Settings,
    pub remote: Settings,
    pub last_stream_id: u32,
    saw_first_settings: bool,
    goaway_sent: bool,
}

impl<S: Read + Write> Connection<S> {
    pub fn new(io: S) -> Self {
        Connection {
            io,
            dec: FrameDecoder::new(),
            local: Settings::default(),
            remote: Settings::default(),
            last_stream_id: 0,
            saw_first_settings: false,
            goaway_sent: false,
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        if let Err(e) = self.handshake() {
            return self.fail(e);
        }
        let mut chunk = [0u8; READ_CHUNK];
        loop {
            let n = self.io.read(&mut chunk)?;
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

    fn dispatch(&mut self, f: Frame) -> H2Result<()> {
        let h = f.header;
        if !self.saw_first_settings && h.kind != FrameType::Settings {
            return Err(H2Error::Connection(ErrorCode::ProtocolError));
        }
        match h.kind {
            FrameType::Settings => {
                if h.stream_id != 0 {
                    return Err(H2Error::Connection(ErrorCode::ProtocolError));
                }
                self.saw_first_settings = true;
                if h.has(flag::ACK) {
                    if h.length != 0 {
                        return Err(H2Error::Connection(ErrorCode::FrameSizeError));
                    }
                    return Ok(());
                }
                let mut next = self.remote;
                next.apply(&f.payload)?;
                self.remote = next;
                let ack = Frame::new(FrameType::Settings, flag::ACK, 0, Vec::new());
                self.write_frame(&ack)
            }
            FrameType::Ping => {
                if h.stream_id != 0 {
                    return Err(H2Error::Connection(ErrorCode::ProtocolError));
                }
                if h.length != 8 {
                    return Err(H2Error::Connection(ErrorCode::FrameSizeError));
                }
                if h.has(flag::ACK) {
                    return Ok(());
                }
                let pong = Frame::new(FrameType::Ping, flag::ACK, 0, f.payload);
                self.write_frame(&pong)
            }
            FrameType::GoAway => {
                if h.stream_id != 0 {
                    return Err(H2Error::Connection(ErrorCode::ProtocolError));
                }
                if h.length < 8 {
                    return Err(H2Error::Connection(ErrorCode::FrameSizeError));
                }
                Err(H2Error::Connection(ErrorCode::NoError))
            }
            FrameType::WindowUpdate => {
                if h.length != 4 {
                    return Err(H2Error::Connection(ErrorCode::FrameSizeError));
                }
                let raw = [f.payload[0], f.payload[1], f.payload[2], f.payload[3]];
                let inc = u32::from_be_bytes(raw) & 0x7fff_ffff;
                if inc == 0 {
                    if h.stream_id == 0 {
                        return Err(H2Error::Connection(ErrorCode::ProtocolError));
                    }
                    return Err(H2Error::Stream {
                        id: h.stream_id,
                        code: ErrorCode::ProtocolError,
                    });
                }
                Ok(())
            }
            FrameType::Priority => {
                if h.stream_id == 0 {
                    return Err(H2Error::Connection(ErrorCode::ProtocolError));
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
                    return Err(H2Error::Connection(ErrorCode::ProtocolError));
                }
                if h.length != 4 {
                    return Err(H2Error::Connection(ErrorCode::FrameSizeError));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn goaway(&mut self, code: ErrorCode) -> io::Result<()> {
        if self.goaway_sent {
            return Ok(());
        }
        self.goaway_sent = true;
        let mut payload = Vec::with_capacity(8);
        payload.extend_from_slice(&(self.last_stream_id & 0x7fff_ffff).to_be_bytes());
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

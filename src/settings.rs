use crate::error::{ErrorCode, H2Error, H2Result};

pub const HEADER_TABLE_SIZE: u16 = 0x1;
pub const ENABLE_PUSH: u16 = 0x2;
pub const MAX_CONCURRENT_STREAMS: u16 = 0x3;
pub const INITIAL_WINDOW_SIZE: u16 = 0x4;
pub const MAX_FRAME_SIZE: u16 = 0x5;
pub const MAX_HEADER_LIST_SIZE: u16 = 0x6;

pub const MAX_WINDOW_SIZE: u32 = 0x7fff_ffff;
pub const MIN_MAX_FRAME_SIZE: u32 = 16_384;
pub const MAX_MAX_FRAME_SIZE: u32 = 16_777_215;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub header_table_size: u32,
    pub enable_push: bool,
    pub max_concurrent_streams: Option<u32>,
    pub initial_window_size: u32,
    pub max_frame_size: u32,
    pub max_header_list_size: Option<u32>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            header_table_size: 4096,
            enable_push: true,
            max_concurrent_streams: None,
            initial_window_size: 65_535,
            max_frame_size: MIN_MAX_FRAME_SIZE,
            max_header_list_size: None,
        }
    }
}

impl Settings {
    pub fn apply(&mut self, payload: &[u8]) -> H2Result<()> {
        if payload.len() % 6 != 0 {
            return Err(H2Error::Connection(ErrorCode::FrameSizeError));
        }
        for chunk in payload.chunks_exact(6) {
            let id = u16::from_be_bytes([chunk[0], chunk[1]]);
            let value = u32::from_be_bytes([chunk[2], chunk[3], chunk[4], chunk[5]]);
            match id {
                HEADER_TABLE_SIZE => self.header_table_size = value,
                ENABLE_PUSH => match value {
                    0 => self.enable_push = false,
                    1 => self.enable_push = true,
                    _ => return Err(H2Error::Connection(ErrorCode::ProtocolError)),
                },
                MAX_CONCURRENT_STREAMS => self.max_concurrent_streams = Some(value),
                INITIAL_WINDOW_SIZE => {
                    if value > MAX_WINDOW_SIZE {
                        return Err(H2Error::Connection(ErrorCode::FlowControlError));
                    }
                    self.initial_window_size = value;
                }
                MAX_FRAME_SIZE => {
                    if !(MIN_MAX_FRAME_SIZE..=MAX_MAX_FRAME_SIZE).contains(&value) {
                        return Err(H2Error::Connection(ErrorCode::ProtocolError));
                    }
                    self.max_frame_size = value;
                }
                MAX_HEADER_LIST_SIZE => self.max_header_list_size = Some(value),
                _ => {}
            }
        }
        Ok(())
    }

    pub fn encode_payload(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut put = |id: u16, v: u32| {
            out.extend_from_slice(&id.to_be_bytes());
            out.extend_from_slice(&v.to_be_bytes());
        };
        put(HEADER_TABLE_SIZE, self.header_table_size);
        put(INITIAL_WINDOW_SIZE, self.initial_window_size);
        put(MAX_FRAME_SIZE, self.max_frame_size);
        if let Some(n) = self.max_concurrent_streams {
            put(MAX_CONCURRENT_STREAMS, n);
        }
        out
    }
}

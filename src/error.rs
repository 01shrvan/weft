#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    NoError,
    ProtocolError,
    InternalError,
    FlowControlError,
    SettingsTimeout,
    StreamClosed,
    FrameSizeError,
    RefusedStream,
    Cancel,
    CompressionError,
    ConnectError,
    EnhanceYourCalm,
    InadequateSecurity,
    Http11Required,
}

impl ErrorCode {
    pub fn as_u32(self) -> u32 {
        match self {
            ErrorCode::NoError => 0x0,
            ErrorCode::ProtocolError => 0x1,
            ErrorCode::InternalError => 0x2,
            ErrorCode::FlowControlError => 0x3,
            ErrorCode::SettingsTimeout => 0x4,
            ErrorCode::StreamClosed => 0x5,
            ErrorCode::FrameSizeError => 0x6,
            ErrorCode::RefusedStream => 0x7,
            ErrorCode::Cancel => 0x8,
            ErrorCode::CompressionError => 0x9,
            ErrorCode::ConnectError => 0xa,
            ErrorCode::EnhanceYourCalm => 0xb,
            ErrorCode::InadequateSecurity => 0xc,
            ErrorCode::Http11Required => 0xd,
        }
    }

    pub fn from_u32(v: u32) -> Self {
        match v {
            0x0 => ErrorCode::NoError,
            0x1 => ErrorCode::ProtocolError,
            0x2 => ErrorCode::InternalError,
            0x3 => ErrorCode::FlowControlError,
            0x4 => ErrorCode::SettingsTimeout,
            0x5 => ErrorCode::StreamClosed,
            0x6 => ErrorCode::FrameSizeError,
            0x7 => ErrorCode::RefusedStream,
            0x8 => ErrorCode::Cancel,
            0x9 => ErrorCode::CompressionError,
            0xa => ErrorCode::ConnectError,
            0xb => ErrorCode::EnhanceYourCalm,
            0xc => ErrorCode::InadequateSecurity,
            0xd => ErrorCode::Http11Required,
            _ => ErrorCode::InternalError,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H2Error {
    Connection(ErrorCode),
    Stream { id: u32, code: ErrorCode },
}

impl H2Error {
    pub fn code(self) -> ErrorCode {
        match self {
            H2Error::Connection(c) => c,
            H2Error::Stream { code, .. } => code,
        }
    }
}

pub type H2Result<T> = Result<T, H2Error>;

use crate::error::{ErrorCode, H2Error, H2Result};
use crate::hpack::table::Header;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Open,
    HalfClosedRemote,
    HalfClosedLocal,
    Closed,
}

#[derive(Debug)]
pub struct Stream {
    pub id: u32,
    pub state: State,
    pub send_window: i64,
    pub content_length: Option<u64>,
    pub data_received: u64,
    pub pending: Vec<u8>,
    pub headers_sent: bool,
    pub finished: bool,
}

impl Stream {
    pub fn new(id: u32, initial_send: i64) -> Self {
        Stream {
            id,
            state: State::Idle,
            send_window: initial_send,
            content_length: None,
            data_received: 0,
            pending: Vec::new(),
            headers_sent: false,
            finished: false,
        }
    }

    pub fn active(&self) -> bool {
        matches!(
            self.state,
            State::Open | State::HalfClosedRemote | State::HalfClosedLocal
        )
    }
}

static CONNECTION_SPECIFIC: [&str; 5] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
];

fn stream_error<T>(id: u32) -> H2Result<T> {
    Err(H2Error::Stream {
        id,
        code: ErrorCode::ProtocolError,
    })
}

#[derive(Debug, Default)]
pub struct Request {
    pub method: Option<Vec<u8>>,
    pub scheme: Option<Vec<u8>>,
    pub path: Option<Vec<u8>>,
    pub authority: Option<Vec<u8>>,
    pub content_length: Option<u64>,
}

fn check_name(id: u32, name: &[u8]) -> H2Result<()> {
    if name.is_empty() {
        return stream_error(id);
    }
    if name.iter().any(|b| b.is_ascii_uppercase()) {
        return stream_error(id);
    }
    Ok(())
}

fn check_regular(id: u32, h: &Header) -> H2Result<()> {
    let name = String::from_utf8_lossy(&h.name).into_owned();
    if CONNECTION_SPECIFIC.contains(&name.as_str()) {
        return stream_error(id);
    }
    if name == "te" && h.value.as_slice() != b"trailers" {
        return stream_error(id);
    }
    Ok(())
}

pub fn validate(id: u32, headers: &[Header]) -> H2Result<Request> {
    let mut req = Request::default();
    let mut seen_regular = false;

    for h in headers {
        check_name(id, &h.name)?;

        if h.name[0] == b':' {
            if seen_regular {
                return stream_error(id);
            }
            let slot = match h.name.as_slice() {
                b":method" => &mut req.method,
                b":scheme" => &mut req.scheme,
                b":path" => &mut req.path,
                b":authority" => &mut req.authority,
                _ => return stream_error(id),
            };
            if slot.is_some() {
                return stream_error(id);
            }
            *slot = Some(h.value.clone());
            continue;
        }

        seen_regular = true;
        check_regular(id, h)?;

        if h.name.as_slice() == b"content-length" {
            let text = String::from_utf8_lossy(&h.value).into_owned();
            let parsed = text.parse::<u64>();
            match parsed {
                Ok(n) => {
                    if req.content_length.is_some() && req.content_length != Some(n) {
                        return stream_error(id);
                    }
                    req.content_length = Some(n);
                }
                Err(_) => return stream_error(id),
            }
        }
    }

    if req.method.is_none() || req.scheme.is_none() || req.path.is_none() {
        return stream_error(id);
    }
    if req.path.as_deref() == Some(b"") {
        return stream_error(id);
    }
    Ok(req)
}

pub fn validate_trailers(id: u32, headers: &[Header]) -> H2Result<()> {
    for h in headers {
        check_name(id, &h.name)?;
        if h.name[0] == b':' {
            return stream_error(id);
        }
        check_regular(id, h)?;
    }
    Ok(())
}

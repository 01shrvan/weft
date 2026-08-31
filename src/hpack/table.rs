use std::collections::VecDeque;

use crate::error::{ErrorCode, H2Error, H2Result};

pub const ENTRY_OVERHEAD: usize = 32;

pub static STATIC_TABLE: [(&str, &str); 61] = [
    (":authority", ""),
    (":method", "GET"),
    (":method", "POST"),
    (":path", "/"),
    (":path", "/index.html"),
    (":scheme", "http"),
    (":scheme", "https"),
    (":status", "200"),
    (":status", "204"),
    (":status", "206"),
    (":status", "304"),
    (":status", "400"),
    (":status", "404"),
    (":status", "500"),
    ("accept-charset", ""),
    ("accept-encoding", "gzip, deflate"),
    ("accept-language", ""),
    ("accept-ranges", ""),
    ("accept", ""),
    ("access-control-allow-origin", ""),
    ("age", ""),
    ("allow", ""),
    ("authorization", ""),
    ("cache-control", ""),
    ("content-disposition", ""),
    ("content-encoding", ""),
    ("content-language", ""),
    ("content-length", ""),
    ("content-location", ""),
    ("content-range", ""),
    ("content-type", ""),
    ("cookie", ""),
    ("date", ""),
    ("etag", ""),
    ("expect", ""),
    ("expires", ""),
    ("from", ""),
    ("host", ""),
    ("if-match", ""),
    ("if-modified-since", ""),
    ("if-none-match", ""),
    ("if-range", ""),
    ("if-unmodified-since", ""),
    ("last-modified", ""),
    ("link", ""),
    ("location", ""),
    ("max-forwards", ""),
    ("proxy-authenticate", ""),
    ("proxy-authorization", ""),
    ("range", ""),
    ("referer", ""),
    ("refresh", ""),
    ("retry-after", ""),
    ("server", ""),
    ("set-cookie", ""),
    ("strict-transport-security", ""),
    ("transfer-encoding", ""),
    ("user-agent", ""),
    ("vary", ""),
    ("via", ""),
    ("www-authenticate", ""),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

impl Header {
    pub fn size(&self) -> usize {
        self.name.len() + self.value.len() + ENTRY_OVERHEAD
    }
}

#[derive(Debug)]
pub struct DynamicTable {
    entries: VecDeque<Header>,
    size: usize,
    max_size: usize,
    hard_limit: usize,
}

impl DynamicTable {
    pub fn new(max_size: usize) -> Self {
        DynamicTable {
            entries: VecDeque::new(),
            size: 0,
            max_size,
            hard_limit: max_size,
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn max_size(&self) -> usize {
        self.max_size
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn set_hard_limit(&mut self, limit: usize) {
        self.hard_limit = limit;
        if self.max_size > limit {
            self.set_max_size(limit).ok();
        }
    }

    pub fn set_max_size(&mut self, new_max: usize) -> H2Result<()> {
        if new_max > self.hard_limit {
            return Err(H2Error::Connection(ErrorCode::CompressionError));
        }
        self.max_size = new_max;
        self.evict_to_fit(0);
        Ok(())
    }

    fn evict_to_fit(&mut self, incoming: usize) {
        while self.size + incoming > self.max_size {
            match self.entries.pop_back() {
                Some(dropped) => self.size -= dropped.size(),
                None => break,
            }
        }
    }

    pub fn insert(&mut self, header: Header) {
        let need = header.size();
        self.evict_to_fit(need);
        if need > self.max_size {
            return;
        }
        self.size += need;
        self.entries.push_front(header);
    }

    pub fn get(&self, index: usize) -> Option<&Header> {
        self.entries.get(index)
    }
}

pub fn lookup(dynamic: &DynamicTable, index: u32) -> H2Result<Header> {
    if index == 0 {
        return Err(H2Error::Connection(ErrorCode::CompressionError));
    }
    let i = index as usize;
    if i <= STATIC_TABLE.len() {
        let (name, value) = STATIC_TABLE[i - 1];
        return Ok(Header {
            name: name.as_bytes().to_vec(),
            value: value.as_bytes().to_vec(),
        });
    }
    match dynamic.get(i - STATIC_TABLE.len() - 1) {
        Some(h) => Ok(h.clone()),
        None => Err(H2Error::Connection(ErrorCode::CompressionError)),
    }
}

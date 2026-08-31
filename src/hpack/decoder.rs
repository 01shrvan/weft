use crate::error::{ErrorCode, H2Error, H2Result};
use crate::hpack::huffman;
use crate::hpack::integer;
use crate::hpack::table::{lookup, DynamicTable, Header};

fn compression_error<T>() -> H2Result<T> {
    Err(H2Error::Connection(ErrorCode::CompressionError))
}

pub struct Decoder {
    table: DynamicTable,
}

impl Decoder {
    pub fn new(max_table_size: usize) -> Self {
        Decoder {
            table: DynamicTable::new(max_table_size),
        }
    }

    pub fn table(&self) -> &DynamicTable {
        &self.table
    }

    pub fn set_settings_limit(&mut self, limit: usize) {
        self.table.set_hard_limit(limit);
    }

    fn read_string(&self, buf: &[u8], pos: usize) -> H2Result<(Vec<u8>, usize)> {
        if pos >= buf.len() {
            return compression_error();
        }
        let huffman_coded = buf[pos] & 0x80 != 0;
        let (len, next) = integer::decode(buf, pos, 7)?;
        let len = len as usize;
        let end = match next.checked_add(len) {
            Some(e) => e,
            None => return compression_error(),
        };
        if end > buf.len() {
            return compression_error();
        }
        let raw = &buf[next..end];
        if huffman_coded {
            Ok((huffman::decode(raw)?, end))
        } else {
            Ok((raw.to_vec(), end))
        }
    }

    fn read_name(&self, buf: &[u8], pos: usize, prefix_bits: u8) -> H2Result<(Vec<u8>, usize)> {
        let (index, next) = integer::decode(buf, pos, prefix_bits)?;
        if index == 0 {
            return self.read_string(buf, next);
        }
        let entry = lookup(&self.table, index)?;
        Ok((entry.name, next))
    }

    pub fn decode(&mut self, block: &[u8]) -> H2Result<Vec<Header>> {
        let mut out = Vec::new();
        let mut pos = 0usize;
        let mut allow_size_update = true;

        while pos < block.len() {
            let byte = block[pos];

            if byte & 0x80 != 0 {
                let (index, next) = integer::decode(block, pos, 7)?;
                if index == 0 {
                    return compression_error();
                }
                out.push(lookup(&self.table, index)?);
                pos = next;
                allow_size_update = false;
                continue;
            }

            if byte & 0xc0 == 0x40 {
                let (name, next) = self.read_name(block, pos, 6)?;
                let (value, after) = self.read_string(block, next)?;
                let header = Header { name, value };
                self.table.insert(header.clone());
                out.push(header);
                pos = after;
                allow_size_update = false;
                continue;
            }

            if byte & 0xe0 == 0x20 {
                if !allow_size_update {
                    return compression_error();
                }
                let (size, next) = integer::decode(block, pos, 5)?;
                self.table.set_max_size(size as usize)?;
                pos = next;
                continue;
            }

            let prefix_bits = 4;
            let (name, next) = self.read_name(block, pos, prefix_bits)?;
            let (value, after) = self.read_string(block, next)?;
            out.push(Header { name, value });
            pos = after;
            allow_size_update = false;
        }

        Ok(out)
    }
}

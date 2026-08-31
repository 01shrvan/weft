use crate::hpack::huffman;
use crate::hpack::integer;

pub fn string(out: &mut Vec<u8>, s: &[u8]) {
    let packed = huffman::encoded_len(s);
    if packed < s.len() {
        integer::encode(out, packed as u32, 7, 0x80);
        out.extend_from_slice(&huffman::encode(s));
    } else {
        integer::encode(out, s.len() as u32, 7, 0x00);
        out.extend_from_slice(s);
    }
}

pub fn indexed(out: &mut Vec<u8>, index: u32) {
    integer::encode(out, index, 7, 0x80);
}

pub fn literal_new_name(out: &mut Vec<u8>, name: &[u8], value: &[u8]) {
    integer::encode(out, 0, 4, 0x00);
    string(out, name);
    string(out, value);
}

pub fn literal_indexed_name(out: &mut Vec<u8>, name_index: u32, value: &[u8]) {
    integer::encode(out, name_index, 4, 0x00);
    string(out, value);
}

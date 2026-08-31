use std::sync::OnceLock;

use crate::error::{ErrorCode, H2Error, H2Result};
use crate::hpack::huffman_table::{EOS, HUFFMAN};

const NONE: i32 = -1;

struct Node {
    children: [i32; 2],
    symbol: i32,
}

struct Trie {
    nodes: Vec<Node>,
}

fn trie() -> &'static Trie {
    static TRIE: OnceLock<Trie> = OnceLock::new();
    TRIE.get_or_init(build)
}

fn build() -> Trie {
    let mut nodes = vec![Node {
        children: [NONE, NONE],
        symbol: NONE,
    }];
    for (symbol, (code, bits)) in HUFFMAN.iter().enumerate() {
        let mut cur = 0usize;
        for i in (0..*bits).rev() {
            let bit = ((code >> i) & 1) as usize;
            let next = nodes[cur].children[bit];
            cur = if next == NONE {
                nodes.push(Node {
                    children: [NONE, NONE],
                    symbol: NONE,
                });
                let created = (nodes.len() - 1) as i32;
                nodes[cur].children[bit] = created;
                created as usize
            } else {
                next as usize
            };
        }
        nodes[cur].symbol = symbol as i32;
    }
    Trie { nodes }
}

fn compression_error<T>() -> H2Result<T> {
    Err(H2Error::Connection(ErrorCode::CompressionError))
}

pub fn encode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut acc: u64 = 0;
    let mut held: u32 = 0;
    for byte in input {
        let (code, bits) = HUFFMAN[*byte as usize];
        acc = (acc << bits) | u64::from(code);
        held += u32::from(bits);
        while held >= 8 {
            held -= 8;
            out.push(((acc >> held) & 0xff) as u8);
        }
    }
    if held > 0 {
        let pad = 8 - held;
        let tail = ((acc << pad) | ((1u64 << pad) - 1)) & 0xff;
        out.push(tail as u8);
    }
    out
}

pub fn encoded_len(input: &[u8]) -> usize {
    let bits: usize = input
        .iter()
        .map(|b| usize::from(HUFFMAN[*b as usize].1))
        .sum();
    bits.div_ceil(8)
}

pub fn decode(input: &[u8]) -> H2Result<Vec<u8>> {
    let t = trie();
    let mut out = Vec::with_capacity(input.len() * 2);
    let mut cur = 0usize;
    let mut partial_bits = 0u32;
    let mut partial_all_ones = true;

    for byte in input {
        for i in (0..8).rev() {
            let bit = ((byte >> i) & 1) as usize;
            if bit == 0 {
                partial_all_ones = false;
            }
            partial_bits += 1;
            let next = t.nodes[cur].children[bit];
            if next == NONE {
                return compression_error();
            }
            cur = next as usize;
            let symbol = t.nodes[cur].symbol;
            if symbol != NONE {
                if symbol as usize == EOS {
                    return compression_error();
                }
                out.push(symbol as u8);
                cur = 0;
                partial_bits = 0;
                partial_all_ones = true;
            }
        }
    }

    if partial_bits >= 8 {
        return compression_error();
    }
    if partial_bits > 0 && !partial_all_ones {
        return compression_error();
    }
    Ok(out)
}

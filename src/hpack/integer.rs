use crate::error::{ErrorCode, H2Error, H2Result};

fn compression_error<T>() -> H2Result<T> {
    Err(H2Error::Connection(ErrorCode::CompressionError))
}

pub fn decode(buf: &[u8], pos: usize, prefix_bits: u8) -> H2Result<(u32, usize)> {
    debug_assert!((1..=8).contains(&prefix_bits));
    if pos >= buf.len() {
        return compression_error();
    }
    let mask = (1u32 << prefix_bits) - 1;
    let first = u32::from(buf[pos]) & mask;
    if first < mask {
        return Ok((first, pos + 1));
    }

    let mut value = u64::from(mask);
    let mut shift = 0u32;
    let mut i = pos + 1;
    loop {
        if i >= buf.len() {
            return compression_error();
        }
        if shift > 28 {
            return compression_error();
        }
        let byte = buf[i];
        value += u64::from(byte & 0x7f) << shift;
        i += 1;
        if value > u64::from(u32::MAX) {
            return compression_error();
        }
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Ok((value as u32, i))
}

pub fn encode(out: &mut Vec<u8>, value: u32, prefix_bits: u8, prefix_value: u8) {
    debug_assert!((1..=8).contains(&prefix_bits));
    let mask = (1u32 << prefix_bits) - 1;
    let keep = prefix_value & !(mask as u8);
    if value < mask {
        out.push(keep | value as u8);
        return;
    }
    out.push(keep | mask as u8);
    let mut rest = value - mask;
    while rest >= 0x80 {
        out.push((rest as u8 & 0x7f) | 0x80);
        rest >>= 7;
    }
    out.push(rest as u8);
}

use weft::error::{ErrorCode, H2Error};
use weft::hpack::integer;
use weft::hpack::table::{lookup, DynamicTable, Header, STATIC_TABLE};

fn h(name: &str, value: &str) -> Header {
    Header {
        name: name.as_bytes().to_vec(),
        value: value.as_bytes().to_vec(),
    }
}

#[test]
fn rfc7541_c11_ten_in_a_five_bit_prefix() {
    let mut out = Vec::new();
    integer::encode(&mut out, 10, 5, 0b1110_0000);
    assert_eq!(out, vec![0b1110_1010]);
    assert_eq!(integer::decode(&out, 0, 5).unwrap(), (10, 1));
}

#[test]
fn rfc7541_c12_1337_in_a_five_bit_prefix() {
    let mut out = Vec::new();
    integer::encode(&mut out, 1337, 5, 0b0000_0000);
    assert_eq!(out, vec![0b0001_1111, 0b1001_1010, 0b0000_1010]);
    assert_eq!(integer::decode(&out, 0, 5).unwrap(), (1337, 3));
}

#[test]
fn rfc7541_c13_forty_two_on_an_octet_boundary() {
    let mut out = Vec::new();
    integer::encode(&mut out, 42, 8, 0);
    assert_eq!(out, vec![0b0010_1010]);
    assert_eq!(integer::decode(&out, 0, 8).unwrap(), (42, 1));
}

#[test]
fn integers_round_trip_across_every_prefix_width() {
    for prefix in 1u8..=8 {
        for value in [0u32, 1, 7, 30, 31, 127, 128, 255, 256, 16_383, 16_384, 1_000_000, u32::MAX - 1] {
            let mut out = Vec::new();
            integer::encode(&mut out, value, prefix, 0);
            let (got, used) = integer::decode(&out, 0, prefix).expect("decodes");
            assert_eq!(got, value, "prefix {prefix} value {value}");
            assert_eq!(used, out.len(), "prefix {prefix} value {value} consumed wrong count");
        }
    }
}

#[test]
fn a_continuation_run_that_overflows_u32_is_a_compression_error() {
    let bomb = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f];
    assert_eq!(
        integer::decode(&bomb, 0, 5).unwrap_err(),
        H2Error::Connection(ErrorCode::CompressionError)
    );
}

#[test]
fn a_truncated_continuation_run_is_a_compression_error() {
    let truncated = [0x1f, 0x9a];
    assert!(integer::decode(&truncated, 0, 5).is_err());
}

#[test]
fn the_static_table_has_the_rfc_endpoints() {
    assert_eq!(STATIC_TABLE.len(), 61);
    assert_eq!(STATIC_TABLE[0], (":authority", ""));
    assert_eq!(STATIC_TABLE[1], (":method", "GET"));
    assert_eq!(STATIC_TABLE[60], ("www-authenticate", ""));
}

#[test]
fn index_zero_and_out_of_range_are_compression_errors() {
    let dt = DynamicTable::new(4096);
    assert_eq!(
        lookup(&dt, 0).unwrap_err(),
        H2Error::Connection(ErrorCode::CompressionError)
    );
    assert!(lookup(&dt, 62).is_err());
    assert_eq!(lookup(&dt, 1).unwrap(), h(":authority", ""));
    assert_eq!(lookup(&dt, 61).unwrap(), h("www-authenticate", ""));
}

#[test]
fn entry_size_is_name_plus_value_plus_thirty_two() {
    let mut dt = DynamicTable::new(4096);
    dt.insert(h("custom-key", "custom-header"));
    assert_eq!(dt.size(), 10 + 13 + 32);
}

#[test]
fn the_newest_entry_is_index_sixty_two() {
    let mut dt = DynamicTable::new(4096);
    dt.insert(h("first", "1"));
    dt.insert(h("second", "2"));
    assert_eq!(lookup(&dt, 62).unwrap(), h("second", "2"));
    assert_eq!(lookup(&dt, 63).unwrap(), h("first", "1"));
}

#[test]
fn inserting_past_the_max_evicts_the_oldest() {
    let mut dt = DynamicTable::new(2 * (5 + 1 + 32));
    dt.insert(h("aaaaa", "1"));
    dt.insert(h("bbbbb", "2"));
    assert_eq!(dt.len(), 2);
    dt.insert(h("ccccc", "3"));
    assert_eq!(dt.len(), 2);
    assert_eq!(lookup(&dt, 62).unwrap(), h("ccccc", "3"));
    assert_eq!(lookup(&dt, 63).unwrap(), h("bbbbb", "2"));
}

#[test]
fn an_entry_larger_than_the_table_empties_it_and_is_not_stored() {
    let mut dt = DynamicTable::new(64);
    dt.insert(h("kept", "x"));
    assert_eq!(dt.len(), 1);
    dt.insert(h("this-name-is-far-too-long-to-ever-fit", "and-so-is-this-value"));
    assert_eq!(dt.len(), 0);
    assert_eq!(dt.size(), 0);
}

#[test]
fn shrinking_the_max_size_evicts_until_it_fits() {
    let mut dt = DynamicTable::new(4096);
    dt.insert(h("aaaaa", "1"));
    dt.insert(h("bbbbb", "2"));
    dt.insert(h("ccccc", "3"));
    assert_eq!(dt.len(), 3);
    dt.set_max_size(38 * 2).unwrap();
    assert_eq!(dt.len(), 2);
    assert_eq!(lookup(&dt, 62).unwrap(), h("ccccc", "3"));
    dt.set_max_size(0).unwrap();
    assert_eq!(dt.len(), 0);
}

#[test]
fn growing_past_the_settings_limit_is_a_compression_error() {
    let mut dt = DynamicTable::new(4096);
    assert_eq!(
        dt.set_max_size(4097).unwrap_err(),
        H2Error::Connection(ErrorCode::CompressionError)
    );
    assert_eq!(dt.max_size(), 4096);
}

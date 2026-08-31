use weft::hpack::decoder::Decoder;
use weft::hpack::table::Header;

fn bytes(hex: &str) -> Vec<u8> {
    let clean: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
        .collect()
}

fn pairs(hs: &[Header]) -> Vec<(String, String)> {
    hs.iter()
        .map(|h| {
            (
                String::from_utf8_lossy(&h.name).into_owned(),
                String::from_utf8_lossy(&h.value).into_owned(),
            )
        })
        .collect()
}

fn expect(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|p| (p.0.to_string(), p.1.to_string()))
        .collect()
}

#[test]
fn rfc7541_c3_request_sequence_without_huffman() {
    let mut d = Decoder::new(4096);

    let first = d.decode(&bytes("828684410f7777772e6578616d706c652e636f6d")).unwrap();
    assert_eq!(
        pairs(&first),
        expect(&[
            (":method", "GET"),
            (":scheme", "http"),
            (":path", "/"),
            (":authority", "www.example.com"),
        ])
    );
    assert_eq!(d.table().size(), 57);

    let second = d.decode(&bytes("828684be58086e6f2d6361636865")).unwrap();
    assert_eq!(
        pairs(&second),
        expect(&[
            (":method", "GET"),
            (":scheme", "http"),
            (":path", "/"),
            (":authority", "www.example.com"),
            ("cache-control", "no-cache"),
        ])
    );
    assert_eq!(d.table().size(), 110);

    let third = d
        .decode(&bytes("828785bf400a637573746f6d2d6b65790c637573746f6d2d76616c7565"))
        .unwrap();
    assert_eq!(
        pairs(&third),
        expect(&[
            (":method", "GET"),
            (":scheme", "https"),
            (":path", "/index.html"),
            (":authority", "www.example.com"),
            ("custom-key", "custom-value"),
        ])
    );
    assert_eq!(d.table().size(), 164);
}

#[test]
fn rfc7541_c4_request_sequence_with_huffman() {
    let mut d = Decoder::new(4096);

    let first = d.decode(&bytes("828684418cf1e3c2e5f23a6ba0ab90f4ff")).unwrap();
    assert_eq!(
        pairs(&first),
        expect(&[
            (":method", "GET"),
            (":scheme", "http"),
            (":path", "/"),
            (":authority", "www.example.com"),
        ])
    );
    assert_eq!(d.table().size(), 57);

    let second = d.decode(&bytes("828684be5886a8eb10649cbf")).unwrap();
    assert_eq!(
        pairs(&second),
        expect(&[
            (":method", "GET"),
            (":scheme", "http"),
            (":path", "/"),
            (":authority", "www.example.com"),
            ("cache-control", "no-cache"),
        ])
    );

    let third = d
        .decode(&bytes("828785bf408825a849e95ba97d7f8925a849e95bb8e8b4bf"))
        .unwrap();
    assert_eq!(
        pairs(&third),
        expect(&[
            (":method", "GET"),
            (":scheme", "https"),
            (":path", "/index.html"),
            (":authority", "www.example.com"),
            ("custom-key", "custom-value"),
        ])
    );
    assert_eq!(d.table().size(), 164);
}

#[test]
fn an_indexed_field_of_zero_is_a_compression_error() {
    let mut d = Decoder::new(4096);
    assert!(d.decode(&[0x80]).is_err());
}

#[test]
fn a_size_update_after_a_header_field_is_a_compression_error() {
    let mut d = Decoder::new(4096);
    assert!(d.decode(&bytes("8220")).is_err(), "size update must be at the block start");
}

#[test]
fn a_size_update_above_the_settings_limit_is_a_compression_error() {
    let mut d = Decoder::new(4096);
    d.set_settings_limit(256);
    assert!(d.decode(&bytes("3fe10f")).is_err());
}

#[test]
fn a_truncated_string_literal_is_a_compression_error() {
    let mut d = Decoder::new(4096);
    assert!(d.decode(&bytes("400a637573746f6d")).is_err());
}

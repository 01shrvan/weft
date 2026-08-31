# weft

An HTTP/2 server written from raw TCP sockets in Rust. No `h2`, no `hyper`, no HPACK
crate. Zero dependencies.

## Score

```
h2spec 2.6.0     146 tests, 146 passed, 0 skipped, 0 failed
cargo test        39 passed
```

## Install

```
cargo install weft-h2
```

The crate is `weft-h2` because `weft` was taken on crates.io in 2018. The binary it
installs is `weft`.

## Run

```
weft 127.0.0.1:8080
```

From a clone:

```
cargo run --release -- 127.0.0.1:8080
```
```
weft listening on 127.0.0.1:8080 (h2c, prior knowledge)
```

Hit it with a real HTTP/2 client:

```
node examples/probe.mjs
```
```
SETTINGS from server: { headerTableSize: 4096, initialWindowSize: 65535, maxFrameSize: 16384, maxConcurrentStreams: 100 }
PING acked in 0.17 ms
response headers: { status: 200, 'content-type': 'text/plain', 'content-length': '5' }
body: "weft\n"
```

Grade it. Binary from [h2spec releases](https://github.com/summerwind/h2spec/releases):

```
h2spec -h 127.0.0.1 -p 8080
```
```
Finished in 0.0660 seconds
146 tests, 146 passed, 0 skipped, 0 failed
```

Tests:

```
cargo test
```

## Implemented

- Frame codec and a streaming decoder that handles frames split across TCP reads
- h2c connection preface, SETTINGS, PING, GOAWAY, RST_STREAM, WINDOW_UPDATE
- HPACK: prefix integers, static table, dynamic table with eviction, Huffman coding
  (table generated from RFC 7541 Appendix B)
- Stream state machine, CONTINUATION assembly, padding, request validation, trailers
- Connection and per-stream flow control, window adjustment on SETTINGS change
- content-length validated against DATA received, MAX_CONCURRENT_STREAMS enforced
- Serves a 200 with an HPACK-encoded header block

## Benchmark

Both servers return an identical response (200, `text/plain`, 5-byte body) over h2c on
loopback. Same client for both, `bench/load.mjs`, 4 connections x 25 concurrent streams,
2s warmup, 8s measured. Two client processes, because one saturates before either server
does.

| server | rps | p99 |
|---|---|---|
| weft | 16,978 | 38.5 ms |
| node `http2` | 30,461 | 13.5 ms |

Node's `http2` is nghttp2 (C) underneath, not JavaScript.

```
cargo run --release -- 127.0.0.1:8080
node bench/node-server.mjs 8081
node bench/load.mjs http://127.0.0.1:8080 4 25 2000 8000
```

Buffering the write path took weft from 8,800 to 11,800 rps on a single client and halved
p99. Before that, every frame was a `write_all` plus a `flush` with `TCP_NODELAY` on, so a
response cost four syscalls and two TCP segments.

## Notes

h2c only, no TLS. Synchronous, one `std::thread` per connection, no tokio.

## License

MIT. See [LICENSE](LICENSE).

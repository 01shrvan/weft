# weft

An HTTP/2 server written from raw TCP sockets in Rust. No `h2`, no `hyper`, no HPACK
crate. Zero runtime dependencies.

`weft` is the crosswise thread woven through a single warp, which is what HTTP/2 does with
streams.

## What this is, and what it isn't

HTTP/2 is a finished, heavily implemented protocol. nghttp2, Node, Go, Netty and curl all
speak it well. **This will never have users, and that is not the point.**

The point is that correctness here is graded by someone else. Every case in
[h2spec](https://github.com/summerwind/h2spec) was written by a stranger against RFC 7540
and RFC 7541. The score cannot be self-scoped, self-assessed, or argued with, which is the
whole reason for building it.

## Score

```
h2spec 2.6.0 | 146 tests, 41 passed, 1 skipped, 104 failed
```

Run against `weft` on 2026-08-30, phase 1 of 5. The 104 failures are real and expected:
HPACK, the stream state machine and flow control are not built yet, and h2spec tests all
three heavily. The number goes here verbatim as it moves, including while it is bad.

Reproduce it:

```
cargo run --release -- 127.0.0.1:8080
h2spec -h 127.0.0.1 -p 8080
```

## Design decisions

**Cleartext h2c, not TLS.** h2spec speaks it and so does
`curl --http2-prior-knowledge`. TLS roughly doubles the work and teaches nothing that
HTTP/2 itself doesn't.

**Synchronous, no tokio.** HTTP/2 multiplexing does not need async. Streams are
multiplexed inside one connection by a state machine on one thread, which is the actual
mental model worth having. One `std::thread` per connection.

**Zero dependencies.** Including the Huffman table, the dynamic table, and the framing.

## Status

Phase 1 of 5 complete. The server listens, completes the h2c handshake and answers frame-level traffic. It cannot yet serve a request.

- [x] frame header codec, streaming decoder, error codes
- [x] connection preface, `SETTINGS` negotiation, `PING`, `GOAWAY`, `RST_STREAM`
- [x] `GOAWAY` on connection errors, `RST_STREAM` on stream errors
- [ ] HPACK
- [ ] stream state machine
- [ ] flow control
- [ ] benchmark against Node `http2`

## Running

```
cargo test
```

## The invariant the decoder has to hold

TCP gives you bytes, not frames. A frame can arrive split across four reads, or five
frames can arrive in one. The test that matters feeds an identical session three ways |
one byte at a time, in 200 randomly chunked runs, and in a single buffer | and asserts all
three decode identically. Chunking is the most common source of real bugs in stream
parsers and it is invisible until it isn't.

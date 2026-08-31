# weft

An HTTP/2 server written from raw TCP sockets in Rust. No `h2`, no `hyper`, no HPACK
crate. Zero runtime dependencies.

`weft` is the crosswise thread woven through a single warp, which is what HTTP/2 does with
streams.

## What this is

HTTP/2 is a finished, heavily implemented protocol. nghttp2, Node, Go, Netty and curl all
speak it well. This is not trying to replace any of them.

It exists because correctness here is graded by someone else. Every case in
[h2spec](https://github.com/summerwind/h2spec) was written by a stranger against RFC 7540
and RFC 7541, so the score cannot be self-scoped or argued with.

## Score

```
h2spec 2.6.0     146 tests, 136 passed, 0 skipped, 10 failed
cargo test       39 passed
```

Both numbers are reproduced by the commands below. They go here verbatim as they move.

## Running it

Build and start the server. It speaks cleartext h2c with prior knowledge, so there is no
TLS and no upgrade dance.

```
cargo run --release -- 127.0.0.1:8080
```

```
weft listening on 127.0.0.1:8080 (h2c, prior knowledge)
```

Probe it with a real HTTP/2 client. `examples/probe.mjs` opens a connection, reads the
server's SETTINGS, sends a PING, then tries a GET:

```
node examples/probe.mjs
```

```
SETTINGS from server: { headerTableSize: 4096, initialWindowSize: 65535, maxFrameSize: 16384, maxConcurrentStreams: 100 }
PING acked in 0.17 ms
response headers: { status: 200, 'content-type': 'text/plain', 'content-length': '5' }
body: "weft
"
```

Grade it against the conformance suite. Download `h2spec` from its
[releases](https://github.com/summerwind/h2spec/releases), then:

```
h2spec -h 127.0.0.1 -p 8080
```

```
Finished in 12.1267 seconds
146 tests, 136 passed, 0 skipped, 10 failed
```

Run the unit and RFC-vector tests:

```
cargo test
```

## What works today

Frame codec and a streaming decoder that survives frames split across TCP reads. The h2c
connection preface, SETTINGS negotiation, PING, GOAWAY on connection errors and
RST_STREAM on stream errors, each with the error code the RFC demands.

HPACK decodes: prefix integers with overflow guards, the static table, a dynamic table
with size-bounded eviction, and Huffman coding whose table is generated directly from
RFC 7541 Appendix B. It decodes both full request sequences from Appendix C, with the
dynamic table reaching exactly the sizes the RFC states.

Requests are served. The stream state machine tracks idle, open, half-closed and closed
with the error code the RFC demands on each illegal transition, header blocks are
assembled across CONTINUATION frames, and requests are validated for pseudo-header
ordering, duplicates, uppercase names, connection-specific fields and `te`.

The ten remaining h2spec failures are five flow-control cases (the send window is not
enforced when writing DATA, and existing stream windows are not adjusted when
SETTINGS_INITIAL_WINDOW_SIZE changes), two content-length mismatch checks, trailers,
MAX_CONCURRENT_STREAMS enforcement, and PRIORITY self-dependency on a standalone frame.

## Design decisions

**Cleartext h2c, not TLS.** h2spec speaks it. TLS roughly doubles the work and teaches
nothing that HTTP/2 itself does not.

**Synchronous, no tokio.** HTTP/2 multiplexing does not need async. Streams are
multiplexed inside one connection by a state machine on one thread, which is the actual
mental model worth having. One `std::thread` per connection.

**Zero dependencies**, including the Huffman table, the dynamic table and the framing.

## The invariant the decoder holds

TCP gives you bytes, not frames. A frame can arrive split across four reads, or five
frames can arrive in one. The test that matters feeds an identical session three ways |
one byte at a time, in 200 randomly chunked runs, and in a single buffer | and asserts all
three decode identically. Chunking is the most common source of real bugs in stream
parsers and it is invisible until it isn't.

## One bug worth recording

The first run against Node's HTTP/2 client failed with `PROTOCOL_ERROR`. RFC 7540 6.5.2
says a server must not set `SETTINGS_ENABLE_PUSH` to 1, and weft was advertising exactly
that. h2spec did not catch it, because h2spec tests how a server answers, not what it
volunteers. Two clients disagree more usefully than one.

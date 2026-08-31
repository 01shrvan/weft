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
h2spec 2.6.0     146 tests, 41 passed, 1 skipped, 104 failed
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
SETTINGS from server: { headerTableSize: 4096, initialWindowSize: 65535, maxFrameSize: 16384 }
PING acked in 0.19 ms
GET / timed out: HEADERS is not handled yet (phase 3)
```

The handshake, SETTINGS negotiation and PING round trip work against a real client. A GET
does not return, because request headers are not wired up yet.

Grade it against the conformance suite. Download `h2spec` from its
[releases](https://github.com/summerwind/h2spec/releases), then:

```
h2spec -h 127.0.0.1 -p 8080
```

```
Finished in 209.5065 seconds
146 tests, 41 passed, 1 skipped, 104 failed
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

Streams, flow control and serving an actual response are not built.

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

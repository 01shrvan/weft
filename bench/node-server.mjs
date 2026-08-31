import http2 from "node:http2";

const port = Number(process.argv[2] ?? 8081);
const BODY = Buffer.from("weft\n");

const server = http2.createServer();
server.on("stream", (stream) => {
  stream.respond({
    ":status": 200,
    "content-type": "text/plain",
    "content-length": String(BODY.length),
  });
  stream.end(BODY);
});
server.on("session", (s) => s.on("error", () => {}));
server.listen(port, "127.0.0.1", () => {
  console.log(`node http2 listening on 127.0.0.1:${port} (h2c)`);
});

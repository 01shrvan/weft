import http2 from "node:http2";

const target = process.argv[2] ?? "http://127.0.0.1:8080";
const done = (code) => {
  process.exit(code);
};
setTimeout(() => {
  console.log("probe timed out after 6s");
  done(1);
}, 6000).unref();

const client = http2.connect(target);

client.on("error", (e) => {
  console.log("connection error:", e.message);
  done(1);
});

client.on("remoteSettings", (s) => {
  console.log("SETTINGS from server:", {
    headerTableSize: s.headerTableSize,
    initialWindowSize: s.initialWindowSize,
    maxFrameSize: s.maxFrameSize,
    maxConcurrentStreams: s.maxConcurrentStreams,
  });

  client.ping((err, duration) => {
    console.log(err ? `PING failed: ${err.message}` : `PING acked in ${duration.toFixed(2)} ms`);

    const req = client.request({ ":method": "GET", ":path": "/" });
    const chunks = [];
    req.on("response", (h) => {
      console.log("response headers:", {
        status: h[":status"],
        "content-type": h["content-type"],
        "content-length": h["content-length"],
      });
    });
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      console.log("body:", JSON.stringify(Buffer.concat(chunks).toString()));
      client.close();
      done(0);
    });
    req.on("error", (e) => {
      console.log("stream error:", e.message);
      done(1);
    });
    req.end();
  });
});

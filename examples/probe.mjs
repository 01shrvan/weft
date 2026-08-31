import http2 from "node:http2";

const target = process.argv[2] ?? "http://127.0.0.1:8080";
const client = http2.connect(target);

client.on("remoteSettings", (s) => {
  console.log("SETTINGS from server:", {
    headerTableSize: s.headerTableSize,
    initialWindowSize: s.initialWindowSize,
    maxFrameSize: s.maxFrameSize,
    enablePush: s.enablePush,
  });
  client.ping((err, duration) => {
    if (err) {
      console.log("PING failed:", err.message);
    } else {
      console.log(`PING acked in ${duration.toFixed(2)} ms`);
    }
    const req = client.request({ ":path": "/" });
    req.setTimeout(2000, () => {
      console.log("GET / timed out: HEADERS is not handled yet (phase 3)");
      client.close();
      process.exit(0);
    });
    req.on("response", (h) => console.log("response:", h));
    req.on("error", (e) => {
      console.log("stream error:", e.message);
      process.exit(0);
    });
    req.end();
  });
});

client.on("error", (e) => {
  console.log("connection error:", e.message);
  process.exit(1);
});

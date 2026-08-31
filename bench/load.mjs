import http2 from "node:http2";

const target = process.argv[2] ?? "http://127.0.0.1:8080";
const connections = Number(process.argv[3] ?? 4);
const concurrency = Number(process.argv[4] ?? 25);
const warmupMs = Number(process.argv[5] ?? 2000);
const durationMs = Number(process.argv[6] ?? 10000);

const latencies = [];
let completed = 0;
let errors = 0;
let measuring = false;
let stopping = false;

function fire(client) {
  if (stopping) return;
  const started = process.hrtime.bigint();
  const req = client.request({ ":method": "GET", ":path": "/" });
  req.on("data", () => {});
  req.on("end", () => {
    if (measuring) {
      latencies.push(Number(process.hrtime.bigint() - started) / 1e6);
      completed++;
    }
    fire(client);
  });
  req.on("error", () => {
    errors++;
    if (!stopping) setTimeout(() => fire(client), 5);
  });
  req.end();
}

function pct(sorted, p) {
  if (sorted.length === 0) return 0;
  const i = Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length));
  return sorted[i];
}

const clients = [];
let ready = 0;

for (let i = 0; i < connections; i++) {
  const c = http2.connect(target, { settings: { enablePush: false } });
  c.on("error", () => { errors++; });
  c.on("connect", () => {
    ready++;
    if (ready === connections) start();
  });
  clients.push(c);
}

function start() {
  for (const c of clients) {
    for (let i = 0; i < concurrency; i++) fire(c);
  }
  setTimeout(() => {
    measuring = true;
    const t0 = process.hrtime.bigint();
    setTimeout(() => {
      const elapsed = Number(process.hrtime.bigint() - t0) / 1e9;
      measuring = false;
      stopping = true;
      const sorted = latencies.slice().sort((a, b) => a - b);
      console.log(JSON.stringify({
        target,
        connections,
        concurrency,
        seconds: Number(elapsed.toFixed(3)),
        requests: completed,
        rps: Math.round(completed / elapsed),
        p50_ms: Number(pct(sorted, 50).toFixed(3)),
        p90_ms: Number(pct(sorted, 90).toFixed(3)),
        p99_ms: Number(pct(sorted, 99).toFixed(3)),
        max_ms: Number(pct(sorted, 100).toFixed(3)),
        errors,
      }));
      for (const c of clients) c.destroy();
      process.exit(0);
    }, durationMs);
  }, warmupMs);
}

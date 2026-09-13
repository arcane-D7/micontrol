/**
 * QA: check latency/hang of get_fan_info & get_audio_volume (the two other
 * legs of fastPoll) plus get_system_info, via MCP TCP socket.
 * If any leg never resolves, fastPollInFlightRef stays true → UI freezes.
 */
import net from 'node:net';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const HOST = '127.0.0.1';
const PORT = 4000;
const tokenPath = path.join(os.tmpdir(), `tauri-mcp-${PORT}.token`);
let AUTH_TOKEN = null;
try {
  AUTH_TOKEN = fs.readFileSync(tokenPath, 'utf8').trim();
} catch {}

function sendCommand(sock, command, payload = {}, timeoutMs = 8000) {
  return new Promise((resolve, reject) => {
    let buffer = '';
    const id = String(Date.now() + Math.random());
    let done = false;
    const finish = (fn, v) => {
      if (!done) {
        done = true;
        clearTimeout(t);
        sock.off('data', handler);
        fn(v);
      }
    };
    const t = setTimeout(() => finish(reject, new Error(`TIMEOUT ${timeoutMs}ms`)), timeoutMs);
    const handler = (chunk) => {
      buffer += chunk.toString('utf8');
      let idx;
      while ((idx = buffer.indexOf('\n')) !== -1) {
        const line = buffer.slice(0, idx).trim();
        buffer = buffer.slice(idx + 1);
        if (!line) continue;
        try {
          const msg = JSON.parse(line);
          if (msg.id === id) finish(resolve, msg);
        } catch {}
      }
    };
    sock.on('data', handler);
    sock.write(JSON.stringify({ command, payload, id, authToken: AUTH_TOKEN }) + '\n');
  });
}

const cases = [
  {
    name: 'get_system_info',
    code: `(async()=>{ const r=await window.__TAURI_INTERNALS__.invoke('get_system_info'); return 'cpu='+r.cpu_usage.toFixed(1); })()`,
  },
  {
    name: 'get_fan_info',
    code: `(async()=>{ const r=await window.__TAURI_INTERNALS__.invoke('get_fan_info'); return 'fans='+(r&&r.fans?r.fans.length:JSON.stringify(r)); })()`,
  },
  {
    name: 'get_audio_volume',
    code: `(async()=>{ const r=await window.__TAURI_INTERNALS__.invoke('get_audio_volume'); return JSON.stringify(r); })()`,
  },
];

const sock = net.connect({ host: HOST, port: PORT }, async () => {
  try {
    for (const c of cases) {
      for (let i = 0; i < 2; i++) {
        const t0 = Date.now();
        const res = await sendCommand(sock, 'execute_js', { code: c.code });
        const ms = Date.now() - t0;
        const out = res && res.data ? res.data.result : JSON.stringify(res).slice(0, 200);
        console.log(`${c.name}#${i} ${ms}ms → ${String(out).slice(0, 160)}`);
      }
    }
  } catch (e) {
    console.error('ERR:', e.message);
    process.exitCode = 1;
  } finally {
    sock.end();
  }
});
sock.on('error', (e) => {
  console.error('Socket error:', e.message);
  process.exitCode = 1;
});

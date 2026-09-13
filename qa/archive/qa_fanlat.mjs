/**
 * QA: precise latency of get_fan_info with a 30s timeout (plugin timeout is 5s,
 * so let's measure the *backend* cost through the JS bridge).
 * Also sample cpu/gpu right after to confirm they keep moving.
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

function sendCommand(sock, command, payload = {}, timeoutMs = 30000) {
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

const sock = net.connect({ host: HOST, port: PORT }, async () => {
  try {
    // 1) get_fan_info with long timeout
    let t0 = Date.now();
    const r1 = await sendCommand(
      sock,
      'execute_js',
      {
        code: `(async()=>{ const r=await window.__TAURI_INTERNALS__.invoke('get_fan_info'); return JSON.stringify({rpm:r.speed_rpm, cpu:r.cpu_temp_celsius, gpu:r.gpu_temp_celsius, mode:r.mode}); })()`,
      },
      30000,
    );
    console.log(
      `get_fan_info → ${Date.now() - t0}ms`,
      r1.error ? `ERR ${r1.error}` : r1.data?.result?.slice(0, 180),
    );

    // 2) immediate second call (cache warm?)
    t0 = Date.now();
    const r2 = await sendCommand(
      sock,
      'execute_js',
      {
        code: `(async()=>{ const r=await window.__TAURI_INTERNALS__.invoke('get_fan_info'); return JSON.stringify({rpm:r.speed_rpm, cpu:r.cpu_temp_celsius, gpu:r.gpu_temp_celsius}); })()`,
      },
      30000,
    );
    console.log(
      `get_fan_info#2 → ${Date.now() - t0}ms`,
      r2.error ? `ERR ${r2.error}` : r2.data?.result?.slice(0, 180),
    );

    // 3) get_system_info immediately (still moving?)
    t0 = Date.now();
    const r3 = await sendCommand(
      sock,
      'execute_js',
      {
        code: `(async()=>{ const r=await window.__TAURI_INTERNALS__.invoke('get_system_info'); return 'cpu='+r.cpu_usage.toFixed(1)+' gpu='+r.gpu_usage.toFixed(1); })()`,
      },
      30000,
    );
    console.log(
      `get_system_info → ${Date.now() - t0}ms`,
      r3.error ? `ERR ${r3.error}` : r3.data?.result,
    );
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

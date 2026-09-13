/**
 * QA: sample get_system_info 5x via MiControl MCP TCP socket (127.0.0.1:4000).
 * Protocol: newline-delimited JSON {command, payload, id, authToken}.
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

function sendCommand(sock, command, payload = {}) {
  return new Promise((resolve, reject) => {
    let buffer = '';
    const id = String(Date.now() + Math.random());
    const handler = (chunk) => {
      buffer += chunk.toString('utf8');
      let idx;
      while ((idx = buffer.indexOf('\n')) !== -1) {
        const line = buffer.slice(0, idx).trim();
        buffer = buffer.slice(idx + 1);
        if (!line) continue;
        try {
          const msg = JSON.parse(line);
          if (msg.id === id) {
            sock.off('data', handler);
            resolve(msg);
          }
        } catch {}
      }
    };
    sock.on('data', handler);
    sock.write(JSON.stringify({ command, payload, id, authToken: AUTH_TOKEN }) + '\n');
    setTimeout(() => {
      sock.off('data', handler);
      reject(new Error('timeout'));
    }, 15000);
  });
}

const code = `(async()=>{ const r = await window.__TAURI_INTERNALS__.invoke('get_system_info'); return JSON.stringify({cpu_usage:r.cpu_usage,gpu_usage:r.gpu_usage,vram_used_mb:r.vram_used_mb,ram_used_gb:r.ram_used_gb,cpu_name:r.cpu_name,gpu_name:r.gpu_name,os_version:r.os_version}); })()`;

const sock = net.connect({ host: HOST, port: PORT }, async () => {
  try {
    for (let i = 0; i < 5; i++) {
      const res = await sendCommand(sock, 'execute_js', { code });
      const out =
        typeof res === 'object' && res.result ? (res.result.returnValue ?? res.result) : res;
      console.log(`sample${i} [${new Date().toISOString().slice(11, 19)}]`, JSON.stringify(out));
      if (i < 4) await new Promise((r) => setTimeout(r, 2500));
    }
  } catch (e) {
    console.error('ERR:', e.message);
    process.exitCode = 1;
  } finally {
    sock.end();
  }
});
const sockErr = sock.on('error', (e) => {
  console.error('Socket error:', e.message);
  process.exitCode = 1;
});

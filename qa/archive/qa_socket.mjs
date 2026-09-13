/**
 * MiControl real-UI QA via tauri-plugin-mcp TCP socket (127.0.0.1:4000).
 * Protocol: newline-delimited JSON {command, payload, id}.
 * Run: node qa_socket.mjs <command> [jsonPayload]
 */
import net from 'node:net';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const HOST = '127.0.0.1';
const PORT = 4000;

// Auth token written by tauri-plugin-mcp to %TEMP%\tauri-mcp-<PORT>.token
const tokenPath = path.join(os.tmpdir(), `tauri-mcp-${PORT}.token`);
let AUTH_TOKEN = null;
try {
  AUTH_TOKEN = fs.readFileSync(tokenPath, 'utf8').trim();
} catch {
  /* no token — server may be unauthenticated on loopback */
}

function sendCommand(sock, command, payload = {}) {
  return new Promise((resolve, reject) => {
    let buffer = '';
    const id = String(Date.now() + Math.random());
    const handler = (chunk) => {
      buffer += chunk.toString('utf8');
      // Keep consuming complete newline-delimited lines
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
        } catch {
          /* partial/keepalive */
        }
      }
    };
    sock.on('data', handler);
    sock.write(JSON.stringify({ command, payload, id, authToken: AUTH_TOKEN }) + '\n');
    setTimeout(() => {
      sock.off('data', handler);
      reject(new Error(`Timeout waiting for response to ${command}`));
    }, 20000);
  });
}

const [, , command, payloadRaw] = process.argv;
let payload = {};
if (payloadRaw) {
  try {
    payload = JSON.parse(payloadRaw);
  } catch {
    payload = { selector: payloadRaw };
  }
}

const sock = net.connect({ host: HOST, port: PORT }, async () => {
  try {
    const res = await sendCommand(sock, command, payload);
    console.log(JSON.stringify(res, null, 2));
  } catch (e) {
    console.error(e.message);
    process.exitCode = 1;
  } finally {
    sock.end();
  }
});
sock.on('error', (e) => {
  console.error('Socket error:', e.message);
  process.exitCode = 1;
});

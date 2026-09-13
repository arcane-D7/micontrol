/** QA: screenshot of main window via MCP (for visual theme check). */
import net from 'node:net';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const PORT = 4000;
const tokenPath = path.join(os.tmpdir(), `tauri-mcp-${PORT}.token`);
let AUTH_TOKEN = null;
try {
  AUTH_TOKEN = fs.readFileSync(tokenPath, 'utf8').trim();
} catch {}

function send(sock, payload) {
  return new Promise((resolve, reject) => {
    let buf = '';
    const id = String(Date.now());
    const h = (c) => {
      buf += c.toString('utf8');
      let i;
      while ((i = buf.indexOf('\n')) !== -1) {
        const line = buf.slice(0, i).trim();
        buf = buf.slice(i + 1);
        if (!line) continue;
        try {
          const m = JSON.parse(line);
          if (m.id === id) {
            sock.off('data', h);
            resolve(m);
          }
        } catch {}
      }
    };
    sock.on('data', h);
    sock.write(
      JSON.stringify({
        command: payload.command,
        payload: payload.payload ?? {},
        id,
        authToken: AUTH_TOKEN,
      }) + '\n',
    );
    setTimeout(() => {
      sock.off('data', h);
      reject(new Error('timeout'));
    }, 20000);
  });
}

const sock = net.connect({ host: '127.0.0.1', port: PORT }, async () => {
  try {
    // Show the main window first (it may be minimized/tray-hidden)
    await send(sock, {
      command: 'manage_window',
      payload: { operation: 'show', window_label: 'main' },
    });
    await new Promise((r) => setTimeout(r, 1200));
    const res = await send(sock, {
      command: 'take_screenshot',
      payload: { windowLabel: 'main', format: 'jpeg', quality: 60 },
    });
    const raw = res?.data?.data ?? res?.data?.image_base64 ?? null;
    const data = raw ? (raw.split(',')[1] ?? raw) : null;
    if (!data) {
      console.log('screenshot keys:', JSON.stringify(res?.data ?? res).slice(0, 400));
    } else {
      fs.writeFileSync('qa_theme_dark.jpeg', Buffer.from(data, 'base64'));
      console.log('saved qa_theme_dark.jpeg', Buffer.from(data, 'base64').length, 'bytes');
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

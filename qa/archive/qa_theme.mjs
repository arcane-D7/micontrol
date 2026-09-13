/**
 * QA: dark-theme contrast validation on the RUNNING 0.2.7-beta app.
 * Reads computed styles of key elements, reports lightness values so we can
 * verify the contrast fixes are live (text-dim 60%, muted 72%, error 70%...).
 */
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

const code = `(async()=>{
  const root = document.documentElement;
  const theme = root.getAttribute('data-theme');
  const cs = getComputedStyle(root);
  const tokens = {};
  for (const t of ['--text','--text-muted','--text-dim','--error','--warning','--accent-contrast','--color-danger','--text-secondary']) {
    tokens[t] = cs.getPropertyValue(t).trim();
  }
  // computed color of a card title + sidebar item + primary button (if present)
  const probe = (sel) => {
    const el = document.querySelector(sel);
    return el ? getComputedStyle(el).color : null;
  };
  return JSON.stringify({
    theme,
    tokens,
    cardTitle: probe('.card-title'),
    statLabel: probe('.stat-label'),
    sidebarItem: probe('.sidebar-item'),
  });
})()`;

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
      JSON.stringify({ command: 'execute_js', payload: { code }, id, authToken: AUTH_TOKEN }) +
        '\n',
    );
    setTimeout(() => {
      sock.off('data', h);
      reject(new Error('timeout'));
    }, 15000);
  });
}

const sock = net.connect({ host: '127.0.0.1', port: PORT }, async () => {
  try {
    const res = await send(sock, {});
    console.log(res?.data?.result ?? JSON.stringify(res).slice(0, 600));
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

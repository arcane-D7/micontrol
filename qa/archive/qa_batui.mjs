/**
 * QA: verify cycle count removed from Battery UI + health tooltip present.
 * Uses the MCP TCP socket (127.0.0.1:4000), newline-delimited JSON protocol.
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
  const rows = [];
  document.querySelectorAll('.stat-row').forEach(r => {
    const l = r.querySelector('.stat-label'), v = r.querySelector('.stat-value');
    if (l && v) rows.push(l.textContent.trim() + '=' + v.textContent.trim());
  });
  const hasCycles = rows.some(x => /cycle|ciclo/i.test(x));
  let healthTooltip = null;
  document.querySelectorAll('[title]').forEach(el => {
    const t = el.getAttribute('title') || '';
    if (/gauge|medidor|reaprende|re-learns|design capacity|original/i.test(t) && !healthTooltip) {
      healthTooltip = t.slice(0, 80);
    }
  });
  return JSON.stringify({ hasCycles, healthTooltip, rows: rows.slice(0, 12) });
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
    const out = res?.data?.result;
    console.log(typeof out === 'string' ? out : JSON.stringify(res, null, 2).slice(0, 800));
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

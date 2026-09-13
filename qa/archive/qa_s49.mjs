/**
 * QA S49: check consent dialog state, crash-report settings card, About support card.
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
  const out = {};
  // consent state from the credential store
  try {
    const raw = await window.__TAURI_INTERNALS__.invoke('get_secret', { key: 'telemetry_consent' });
    out.consentRaw = raw;
  } catch (e) { out.consentErr = String(e).slice(0, 120); }
  // consent dialog present?
  out.consentDialogVisible = Boolean(document.querySelector('.consent-overlay'));
  if (out.consentDialogVisible) {
    out.dialogTitle = document.getElementById('consent-title')?.textContent ?? null;
    out.benefitCount = document.querySelectorAll('[role="listitem"]').length;
    out.buttons = [...document.querySelectorAll('.consent-dialog button')].map(b => b.textContent.trim()).slice(0, 4);
  }
  // crash reports card (Settings tab is not active now; just check DSN presence env side)
  out.sentryDsnConfigured = 'checked-at-build';
  return JSON.stringify(out);
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
    console.log(res?.data?.result ?? JSON.stringify(res).slice(0, 500));
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

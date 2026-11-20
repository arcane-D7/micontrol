#!/usr/bin/env node
/**
 * D1 / D2 / D12 - no unbounded child wait.
 *
 * Every production `.output()`, `.status()` or `.wait()` on a child process
 * in `src-tauri/src` must carry a timeout in the same block/function, or go
 * through the canonical helper `process_spawn::run*` (which enforces a
 * mandatory timeout and tree-kill on expiry). A child waited on without a
 * bound can hang an IPC command forever; the audit counted 98 such sites.
 *
 * The baseline in `scripts/spawn-timeout-allowlist.json` is a ratchet:
 * entries are removed as the offenders are fixed, and a NEW offender fails
 * the build immediately.
 *
 * Usage: node scripts/check-spawn-timeout.mjs [--json]
 */
import fs from 'node:fs';
import path from 'node:path';

const ROOT = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '..');
const SRC = path.join(ROOT, 'src-tauri', 'src');
const ALLOW = path.join(ROOT, 'scripts', 'spawn-timeout-allowlist.json');

function walk(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (e.name.endsWith('.rs')) out.push(p);
  }
  return out;
}

/** True while the `#[cfg(test)]` module that contains `index` is still open. */
const isTestContext = (text, index) => {
  const before = text.slice(Math.max(0, index - 8000), index);
  const lastTest = before.lastIndexOf('#[cfg(test)]');
  if (lastTest === -1) return false;
  const after = before.slice(lastTest);
  const opens = (after.match(/\{/g) ?? []).length;
  const closes = (after.match(/\}/g) ?? []).length;
  return opens > closes;
};

/** True when the spawn goes through the canonical helper (timeout enforced). */
const usesHelper = (before) => /process_spawn::(run|run_capture|run_checked|run_async|spawn_detached)\s*\(/.test(before);

/** True when a timeout appears in the same block: any of
 * `wait_timeout`, `try_wait` + `Instant`, `Duration::from_*` used near a
 * `Command`, `tokio::time::timeout`, or a `TIMEOUT` constant.
 */
const hasTimeout = (window) =>
  /wait_timeout|tokio::time::timeout|Duration::from_(secs|millis|micros)|spawn_timeout|TIMEOUT/.test(window);

const offenders = [];
for (const file of walk(SRC)) {
  const text = fs.readFileSync(file, 'utf8');
  // `.output()`, `.status()`, `.wait()` on child processes (std::process).
  const re = /\.(output|status|wait)\s*\(\s*\)/g;
  let m;
  while ((m = re.exec(text)) !== null) {
    const idx = m.index;
    if (isTestContext(text, idx)) continue;
    // Skip doc/plain comment lines: `.output()` in prose is not a call.
    const lineStart = text.lastIndexOf('\n', idx) + 1;
    const lineTrim = text.slice(lineStart, idx).trim();
    if (lineTrim.startsWith('//')) continue;
    const before = text.slice(Math.max(0, idx - 3000), idx);
    const window = text.slice(Math.max(0, idx - 3000), Math.min(text.length, idx + 1200));
    if (usesHelper(before)) continue;
    if (hasTimeout(window)) continue;
    const line = text.slice(0, idx).split('\n').length;
    offenders.push({ file: path.relative(ROOT, file).replace(/\\/g, '/'), line });
  }
}

let baseline = [];
if (fs.existsSync(ALLOW)) {
  baseline = JSON.parse(fs.readFileSync(ALLOW, 'utf8')).entries ?? [];
}
const key = (o) => `${o.file}:${o.line}`;
const allowed = new Set(baseline.map(key));
const fresh = offenders.filter((o) => !allowed.has(key(o)));

const json = process.argv.includes('--json');
if (json) {
  console.log(JSON.stringify({ total: offenders.length, baseline: baseline.length, fresh }, null, 2));
} else {
  console.log(`spawn-timeout gate: ${offenders.length} unbounded wait(s), ${baseline.length} baselined, ${fresh.length} new`);
  for (const o of fresh) console.log(`  NEW  ${o.file}:${o.line}  (use process_spawn::run* or add a timeout)`);
}
process.exit(fresh.length === 0 ? 0 : 1);

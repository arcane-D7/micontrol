#!/usr/bin/env node
/**
 * C1b / U4: `schtasks` is a single port.
 *
 * Every scheduled-task operation must go through
 * `src-tauri/src/util/schtasks.rs`: one place for the spawn policy
 * (`process_spawn::run_capture`, mandatory timeout, `CREATE_NO_WINDOW`),
 * one place for the XML/CSV parsing and one place for the catalogued
 * errors. A `schtasks` spawn anywhere else re-opens the class of bugs the
 * port exists to close (per-command console flashes, unbounded output,
 * uncatalogued error strings, the L-01 heal loop).
 *
 * The allowlist (`scripts/schtasks-call-allowlist.json`) starts EMPTY and
 * must stay empty: fix the call site, never baseline it.
 *
 * Usage: node scripts/check-schtasks-calls.mjs [--json] [--update]
 */
import fs from 'node:fs';
import path from 'node:path';

const ROOT = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '..');
const SRC = path.join(ROOT, 'src-tauri', 'src');
// Forward slashes on purpose: offender paths are normalized to '/' below.
const PORT = 'src-tauri/src/util/schtasks.rs';
const ALLOW = path.join(ROOT, 'scripts', 'schtasks-call-allowlist.json');

function walk(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (e.name.endsWith('.rs')) out.push(p);
  }
  return out;
}

// Spawn-shaped occurrences only: prose/comments that mention schtasks are
// fine; a Command construction with the schtasks program is not.
const RE = /new\s*\(\s*"(schtasks|schtasks\\.exe)"/g;

const offenders = [];
for (const file of walk(SRC)) {
  const rel = path.relative(ROOT, file).replace(/\\/g, '/');
  if (rel === PORT) continue;
  const text = fs.readFileSync(file, 'utf8');
  let m;
  while ((m = RE.exec(text)) !== null) {
    const line = text.slice(0, m.index).split('\n').length;
    offenders.push({ file: rel, line, match: m[0] });
  }
}

const baseline = JSON.parse(fs.readFileSync(ALLOW, 'utf8')).entries ?? [];
const key = (o) => `${o.file}:${o.line}`;
const allowed = new Set(baseline.map(key));
const fresh = offenders.filter((o) => !allowed.has(key(o)));

const json = process.argv.includes('--json');
const update = process.argv.includes('--update');

if (update) {
  // Regenerate the baseline from the live scan (house ratchet rule: only
  // this gate's own --update may write it).
  fs.writeFileSync(ALLOW, `${JSON.stringify({ entries: offenders }, null, 2)}\n`);
  console.log(`schtasks-call gate: baseline regenerated with ${offenders.length} entr(y/ies)`);
} else if (json) {
  console.log(JSON.stringify({ total: offenders.length, baseline: baseline.length, fresh, allowlistEmpty: baseline.length === 0 }, null, 2));
} else {
  console.log(`schtasks-call gate: ${offenders.length} outside the port, ${baseline.length} baselined, ${fresh.length} new (allowlist must stay empty)`);
  for (const o of fresh) console.log(`  NEW  ${o.file}:${o.line}  (${o.match}) move the call into src-tauri/src/util/schtasks.rs`);
}
process.exit(fresh.length === 0 ? 0 : 1);

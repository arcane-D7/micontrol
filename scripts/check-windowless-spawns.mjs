#!/usr/bin/env node
/**
 * B7 / RNF-14 — no console window may flash.
 *
 * Every production `Command::new(...)` in `src-tauri/src` must create its
 * process without a console: either the builder carries
 * `.creation_flags(CREATE_NO_WINDOW)` (0x08000000), or the spawn goes through
 * the canonical helper that applies it. A spawn that forgets it shows a black
 * console for a few hundred ms, which the owner forbids.
 *
 * The baseline in `scripts/windowless-spawn-allowlist.json` is a ratchet:
 * entries are removed as the offenders are fixed, and a NEW offender fails the
 * build immediately.
 *
 * Usage: node scripts/check-windowless-spawns.mjs [--json]
 */
import fs from 'node:fs';
import path from 'node:path';

const ROOT = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '..');
const SRC = path.join(ROOT, 'src-tauri', 'src');
const ALLOW = path.join(ROOT, 'scripts', 'windowless-spawn-allowlist.json');

function walk(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (e.name.endsWith('.rs')) out.push(p);
  }
  return out;
}

const isTestContext = (text, index) => {
  const before = text.slice(Math.max(0, index - 8000), index);
  const lastTest = before.lastIndexOf('#[cfg(test)]');
  if (lastTest === -1) return false;
  // crude but effective: the `#[cfg(test)]` module must still be open (no closing brace at column 0 after it)
  const after = before.slice(lastTest);
  const opens = (after.match(/\{/g) ?? []).length;
  const closes = (after.match(/\}/g) ?? []).length;
  return opens > closes;
};

const offenders = [];
for (const file of walk(SRC)) {
  const text = fs.readFileSync(file, 'utf8');
  const re = /Command::new\s*\(/g;
  let m;
  while ((m = re.exec(text)) !== null) {
    const idx = m.index;
    if (isTestContext(text, idx)) continue;
    const window = text.slice(Math.max(0, idx - 600), Math.min(text.length, idx + 900));
    const guarded = window.includes('CREATE_NO_WINDOW') || window.includes('creation_flags') || window.includes('spawn_hidden') || window.includes('process_spawn::');
    if (!guarded) {
      const line = text.slice(0, idx).split('\n').length;
      offenders.push({ file: path.relative(ROOT, file).replace(/\\/g, '/'), line });
    }
  }
}

const baseline = JSON.parse(fs.readFileSync(ALLOW, 'utf8')).entries ?? [];
const key = (o) => `${o.file}:${o.line}`;
const allowed = new Set(baseline.map(key));
const fresh = offenders.filter((o) => !allowed.has(key(o)));

const json = process.argv.includes('--json');
if (json) {
  console.log(JSON.stringify({ total: offenders.length, baseline: baseline.length, fresh }, null, 2));
} else {
  console.log(`windowless-spawn gate: ${offenders.length} unguarded spawn(s), ${baseline.length} baselined, ${fresh.length} new`);
  for (const o of fresh) console.log(`  NEW  ${o.file}:${o.line}  (use .creation_flags(CREATE_NO_WINDOW) / spawn_hidden)`);
}
process.exit(fresh.length === 0 ? 0 : 1);

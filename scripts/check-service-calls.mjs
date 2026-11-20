#!/usr/bin/env node
/**
 * C1a / U3 (architecture governor D3) - no sc.exe spawns in production.
 *
 * Every Windows service operation (query, start, stop, create, delete,
 * failure actions, binPath change, state wait) must go through the native
 * SCM API in `src-tauri/src/util/win_service.rs`. Spawning the `sc` binary
 * loses structured errors (exit-code plus text parsing), costs a process
 * per call, and cannot expose the pid (`sc queryex`) or the config verbatim.
 *
 * The gate greps every `.rs` file under `src-tauri/src` for
 * `Command::new("sc")` / `Command::new("sc.exe")` spawns outside
 * `#[cfg(test)]` modules. Test code is excluded with the same brace heuristic
 * the other gates use. The baseline in `scripts/service-calls-allowlist.json`
 * is a ratchet holding ZERO entries: any offender fails the build immediately.
 *
 * Usage: node scripts/check-service-calls.mjs [--json]
 */
import fs from 'node:fs';
import path from 'node:path';

const ROOT = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '..');
const SRC = path.join(ROOT, 'src-tauri', 'src');
const ALLOW = path.join(ROOT, 'scripts', 'service-calls-allowlist.json');

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

// Match every way production code can spawn the sc binary. The older
// `let mut cmd = Command::new("sc.exe")` + run_capture shape is covered by
// the same pattern; comments mentioning sc query etc. never match.
const PATTERNS = [/Command::new\s*\(\s*["']sc\.exe["']\s*\)/g, /Command::new\s*\(\s*["']sc["']\s*\)/g];

const offenders = [];
for (const file of walk(SRC)) {
  const text = fs.readFileSync(file, 'utf8');
  for (const re of PATTERNS) {
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(text)) !== null) {
      const idx = m.index;
      if (isTestContext(text, idx)) continue;
      const line = text.slice(0, idx).split('\n').length;
      offenders.push({
        file: path.relative(ROOT, file).replace(/\\/g, '/'),
        line,
        snippet: text.slice(idx, idx + 60).replace(/\s+/g, ' ').trim(),
      });
    }
  }
}

let baseline = { entries: [] };
if (fs.existsSync(ALLOW)) {
  baseline = JSON.parse(fs.readFileSync(ALLOW, 'utf8'));
} else {
  console.error(`allowlist missing: ${ALLOW}`);
  process.exit(1);
}
const key = (o) => `${o.file}:${o.line}`;
const allowed = new Set(baseline.entries.map(key));
const fresh = offenders.filter((o) => !allowed.has(key(o)));

const json = process.argv.includes('--json');
if (json) {
  console.log(JSON.stringify({ total: offenders.length, baseline: baseline.entries.length, fresh }, null, 2));
} else {
  console.log(`service-calls gate: ${offenders.length} sc.exe spawn(s) outside util::win_service, ${baseline.entries.length} baselined, ${fresh.length} new`);
  for (const o of fresh) console.log(`  NEW  ${o.file}:${o.line}  ${o.snippet}`);
}
process.exit(fresh.length === 0 ? 0 : 1);

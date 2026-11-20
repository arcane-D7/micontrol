#!/usr/bin/env node
/**
 * C0c - State-write ratchet (bounded ramp toward `util::state_store`).
 *
 * Static inventory of direct state writes (`fs::write` / `File::create`) over
 * the Rust sources under `src-tauri/src` (recursive), compared against the
 * committed baseline `scripts/state-writes-baseline.json`.
 *
 * Why: UNIVERSAL-CONTRACTS.md section 3 requires every persisted state to go
 * through `util::state_store.rs` (atomic tmp+rename, fsync, declared schema,
 * ACL). The codebase still has ~29 files writing state directly; this gate
 * does NOT fix them (that is the C3 migration), it freezes them: a new file
 * with direct writes, or a new call site in an existing file, fails the gate.
 * Fixing a site lowers its count - lower always passes.
 *
 * NOT wired into health-check.mjs yet: the parent agent decides when the
 * ratchet turns mandatory (run it with `node scripts/check-state-writes.mjs`).
 *
 * Usage:
 *   node scripts/check-state-writes.mjs            # gate (exit 1 on growth)
 *   node scripts/check-state-writes.mjs --report   # print the full inventory
 *   node scripts/check-state-writes.mjs --update   # regenerate the baseline
 */

import { readFileSync, writeFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '..');
const SRC = path.join(ROOT, 'src-tauri', 'src');
const BASELINE = path.join(__dirname, 'state-writes-baseline.json');

/** Direct write call sites policed by this gate. */
const PATTERNS = [/\bfs::write\s*\(/g, /\bFile::create\s*\(/g];

const argUpdate = process.argv.includes('--update');
const argReport = process.argv.includes('--report');

/** Recursively collect `.rs` files under `dir`. */
function collectRustFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const full = path.join(dir, entry);
    const st = statSync(full);
    if (st.isDirectory()) {
      out.push(...collectRustFiles(full));
    } else if (entry.endsWith('.rs')) {
      out.push(full);
    }
  }
  return out;
}

/**
 * True when `index` sits inside a `#[cfg(test)]` module: tests may create
 * temp files freely (they are not production state), and counting them made
 * the ratchet fire on util/process_spawn.rs's own truncation test.
 */
function isTestContext(text, index) {
  const before = text.slice(0, index);
  const last = before.lastIndexOf('#[cfg(test)]');
  if (last === -1) return false;
  const after = before.slice(last);
  const opens = (after.match(/\{/g) ?? []).length;
  const closes = (after.match(/\}/g) ?? []).length;
  return opens > closes;
}

/** Count gate-pattern matches per file, keyed by repo-relative path. */
function inventory() {
  const counts = {};
  for (const file of collectRustFiles(SRC)) {
    const text = readFileSync(file, 'utf8');
    let n = 0;
    for (const pattern of PATTERNS) {
      pattern.lastIndex = 0;
      let m;
      while ((m = pattern.exec(text)) !== null) {
        if (!isTestContext(text, m.index)) n += 1;
      }
    }
    if (n > 0) {
      counts[path.relative(ROOT, file).replaceAll('\\', '/')] = n;
    }
  }
  return counts;
}

const current = inventory();

if (argUpdate) {
  const sorted = Object.fromEntries(
    Object.entries(current).sort(([a], [b]) => a.localeCompare(b)),
  );
  const total = Object.values(sorted).reduce((sum, n) => sum + n, 0);
  writeFileSync(
    BASELINE,
    `${JSON.stringify({ comment: 'Per-file counts of fs::write/File::create under src-tauri/src. Ratchet baseline for scripts/check-state-writes.mjs; regenerate with --update.', total, files: sorted }, null, 2)}\n`,
  );
  console.log(`baseline updated: ${Object.keys(sorted).length} files, ${total} call sites`);
  process.exit(0);
}

let baseline = { total: 0, files: {} };
try {
  baseline = JSON.parse(readFileSync(BASELINE, 'utf8'));
} catch {
  console.error(`baseline missing: run "node scripts/check-state-writes.mjs --update" first`);
  process.exit(1);
}

const violations = [];
const names = new Set([...Object.keys(current), ...Object.keys(baseline.files)]);
for (const name of [...names].sort()) {
  const before = baseline.files[name] ?? 0;
  const after = current[name] ?? 0;
  if (after > before) {
    violations.push(`${name}: ${before} -> ${after} (new direct state writes)`);
  } else if (after < before) {
    console.log(`${name}: ${after} (improved: ${before} -> ${after})`);
  } else if (argReport) {
    console.log(`${name}: ${after}`);
  }
}

const currentTotal = Object.values(current).reduce((sum, n) => sum + n, 0);
if (argReport) {
  console.log(`total: ${currentTotal} direct call sites in ${Object.keys(current).length} files`);
}

if (violations.length > 0) {
  console.error(`\nstate-write ratchet FAILED (${violations.length}):`);
  for (const line of violations) {
    console.error(`  - ${line}`);
  }
  console.error('\nPersist state through util::state_store (Store) instead of fs::write/File::create.');
  process.exit(1);
}

console.log(
  `state-write ratchet OK: ${currentTotal} call sites (baseline ${baseline.total}), no growth`,
);

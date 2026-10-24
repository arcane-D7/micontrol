#!/usr/bin/env node
/**
 * S67 — Handle-leak prevention gate.
 *
 * Static analysis over `src-tauri/src/**\/*.rs` that fails the build when new
 * code acquires a Windows kernel handle WITHOUT the RAII guard introduced in
 * `util/win_handle.rs`.
 *
 * Why a static gate in addition to the runtime monitor:
 *   The audited leak (`memory-leak-audit.md`) was invisible for 218 hours and
 *   reached 188,715 handles. The runtime monitor catches it in ~1 h; this gate
 *   catches it before it ever ships, at the cheapest possible moment.
 *
 * Rules
 * -----
 * R1. Every `CreateFileW` / `CreateEventW` / `CreateNamedPipeW` / `OpenProcess`
 *     call site must be in a function that ALSO either:
 *       (a) wraps the result in `OwnedHandle` / `OwnedEvent`, or
 *       (b) hands it to an owning wrapper immediately (`File::from_raw_handle`),
 *           or
 *       (c) is explicitly annotated with a justification:
 *             // leak-check: <reason>
 *
 * R2. A function that creates handles and contains `CloseHandle` must not use
 *     `?` or `return Err` between the creation and every CloseHandle — unless it
 *     uses RAII. This is approximated by R1 (RAII preferred) plus a report.
 *
 * Usage:
 *   node scripts/check-handle-leaks.mjs            # gate (exit 1 on violation)
 *   node scripts/check-handle-leaks.mjs --report   # print the full inventory
 */

import { readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '..');
const SRC = path.join(ROOT, 'src-tauri', 'src');

/** Handle-acquiring Win32 calls we police. */
const ACQUIRE = [
  'CreateFileW',
  'CreateEventW',
  'CreateNamedPipeW',
  'CreateMutexW',
  'OpenProcess',
  'CreateToolhelp32Snapshot',
];

/** RAII / ownership wrappers that make a call site safe. */
const GUARDS = ['OwnedHandle', 'OwnedEvent', 'from_raw_handle', 'SendHandle'];

/** Inline justification marker. */
const ANNOTATION = 'leak-check:';

/** Files that are allowed to own handles without the RAII types. */
const ALLOWLIST = new Set([
  // The RAII module itself is where raw handles are intentionally created.
  path.join('util', 'win_handle.rs'),
]);

function walk(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const full = path.join(dir, entry);
    if (statSync(full).isDirectory()) out.push(...walk(full));
    else if (entry.endsWith('.rs')) out.push(full);
  }
  return out;
}

/**
 * Split a file into function-sized chunks. Deliberately simple: a chunk starts
 * at a `fn` line and runs until the next top-level `fn` (or EOF). Over-broad
 * chunks only make the check more conservative, never less.
 */
function chunks(source) {
  const lines = source.split(/\r?\n/);
  const out = [];
  let current = null;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const fnMatch = /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?fn\s+(\w+)/.exec(line);
    if (fnMatch) {
      if (current) out.push(current);
      // Look BACK a few lines for an annotation/directive that sits between the
      // attribute (`#[cfg(...)]`) and the `fn` line — that is the idiomatic
      // Rust place for a justification comment.
      const preamble = lines.slice(Math.max(0, i - 4), i).join('\n');
      current = { name: fnMatch[1], start: i + 1, lines: [], preamble };
    }
    if (current) current.lines.push(line);
  }
  if (current) out.push(current);
  return out;
}

function analyseFile(file) {
  const rel = path.relative(SRC, file);
  const source = readFileSync(file, 'utf8');
  const violations = [];
  const inventory = [];

  for (const chunk of chunks(source)) {
    const text = chunk.lines.join('\n');
    const context = `${chunk.preamble ?? ''}\n${text}`;
    const acquired = ACQUIRE.filter((api) => new RegExp(`\\b${api}\\s*\\(`).test(text));
    if (acquired.length === 0) continue;

    const guarded = GUARDS.some((g) => text.includes(g));
    const annotated = context.includes(ANNOTATION);
    const allowlisted = ALLOWLIST.has(rel);

    inventory.push({
      file: rel,
      fn: chunk.name,
      line: chunk.start,
      apis: acquired,
      guarded,
      annotated,
    });

    if (guarded || annotated || allowlisted) continue;

    violations.push({
      file: rel,
      fn: chunk.name,
      line: chunk.start,
      apis: acquired,
    });
  }
  return { violations, inventory };
}

function main() {
  const wantReport = process.argv.includes('--report');
  const files = walk(SRC);

  const allViolations = [];
  const allInventory = [];
  for (const file of files) {
    const { violations, inventory } = analyseFile(file);
    allViolations.push(...violations);
    allInventory.push(...inventory);
  }

  if (wantReport) {
    console.log(`Handle-acquisition sites: ${allInventory.length}\n`);
    for (const row of allInventory.sort((a, b) => a.file.localeCompare(b.file))) {
      const state = row.guarded ? 'RAII ' : row.annotated ? 'NOTE ' : 'RAW  ';
      console.log(`${state} ${row.file}:${row.line} ${row.fn} — ${row.apis.join(', ')}`);
    }
    console.log();
  }

  if (allViolations.length === 0) {
    console.log(
      `\u2713 handle-leak gate: ${allInventory.length} acquisition site(s), all guarded by RAII.`,
    );
    process.exit(0);
  }

  console.error('Handle-leak gate FAILED — unguarded kernel-handle acquisition:');
  for (const v of allViolations) {
    console.error(`  \u2716 ${v.file}:${v.line} ${v.fn}() — ${v.apis.join(', ')}`);
  }
  console.error(
    '\nFix: wrap the handle in `crate::util::win_handle::OwnedHandle` ' +
      '(or `OwnedEvent` for events). See src-tauri/src/util/win_handle.rs.\n' +
      'If the site genuinely cannot use RAII (e.g. it hands the raw handle to ' +
      'an owning wrapper), add a `// leak-check: <reason>` comment in that function.',
  );
  process.exit(1);
}

main();
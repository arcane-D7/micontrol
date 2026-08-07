#!/usr/bin/env node
/**
 * Health check — single source of truth for local dev and CI.
 *
 * Local:   npm run health-check
 * CI:      ci.yml / release.yml invoke this exact script, so the pipeline
 *          can never drift from what you run locally.
 *
 * Usage:
 *   node scripts/health-check.mjs                   # run every check
 *   node scripts/health-check.mjs --step=a,b,c      # run only these steps
 *   node scripts/health-check.mjs --skip=d,e        # run all but these
 *   node scripts/health-check.mjs --list            # list available steps
 */
import { spawnSync } from 'node:child_process';
import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import process from 'node:process';

const ROOT = path.resolve(import.meta.dirname, '..');

const STEPS = [
  { id: 'version', name: 'Version consistency check', cmd: 'npm', args: ['run', 'version:check'] },
  { id: 'fmt', name: 'Rust formatting check', cmd: 'cargo', args: ['fmt', '--check'], cwd: 'src-tauri' },
  { id: 'clippy', name: 'Rust clippy (warnings denied)', cmd: 'cargo', args: ['clippy', '--', '-D', 'warnings'], cwd: 'src-tauri' },
  { id: 'test-rust', name: 'Rust tests', cmd: 'cargo', args: ['test', '--', '--test-threads=1'], cwd: 'src-tauri', env: { RUST_TEST_THREADS: '1' } },
  { id: 'tsc', name: 'TypeScript type check', cmd: 'npx', args: ['tsc', '--noEmit'] },
  { id: 'lint', name: 'ESLint', cmd: 'npm', args: ['run', 'lint'] },
  { id: 'format', name: 'Prettier check', cmd: 'npm', args: ['run', 'format:check'] },
  { id: 'test-front', name: 'Frontend tests + coverage', cmd: 'npx', args: ['vitest', 'run', '--coverage'] },
  { id: 'build', name: 'Frontend build', cmd: 'npm', args: ['run', 'build'] },
  { id: 'i18n', name: 'i18n completeness check', cmd: 'node', args: ['scripts/check-i18n.mjs'] },
  { id: 'toolchain', name: 'Toolchain consistency check', fn: checkToolchain },
];

// ---------------------------------------------------------------------------
// Toolchain consistency: guarantees local (Node/Rust) never drifts from CI.
// ---------------------------------------------------------------------------
function checkToolchain() {
  const problems = [];

  const nvmrc = readFileSync(path.join(ROOT, '.nvmrc'), 'utf8').trim();
  if (!/^\d+\.\d+\.\d+$/.test(nvmrc)) {
    problems.push(`.nvmrc must pin an exact Node version (e.g. 24.15.0) — found "${nvmrc}"`);
  }

  const pkg = JSON.parse(readFileSync(path.join(ROOT, 'package.json'), 'utf8'));
  const engine = pkg.engines?.node;
  if (!engine) {
    problems.push('package.json is missing "engines.node" — add it so CI/local cannot drift');
  } else {
    const major = nvmrc.split('.')[0];
    const covers = new RegExp(`(^|[^0-9(])(>=|\\^|~)?${major}([.\\s]|$)`).test(engine);
    if (!covers) {
      problems.push(`.nvmrc (${nvmrc}) is not covered by engines.node (${engine})`);
    }
  }

  const rtPath = path.join(ROOT, 'src-tauri', 'rust-toolchain.toml');
  const rt = readFileSync(rtPath, 'utf8');
  if (!/channel\s*=\s*["']\d+\.\d+\.\d+["']/.test(rt)) {
    problems.push('src-tauri/rust-toolchain.toml must pin an exact channel (e.g. channel = "1.95.0")');
  }

  const wfDir = path.join(ROOT, '.github', 'workflows');
  for (const f of readdirSync(wfDir).filter((f) => f.endsWith('.yml') || f.endsWith('.yaml'))) {
    const content = readFileSync(path.join(wfDir, f), 'utf8');
    if (/node-version:\s*['"]?\d/.test(content)) {
      problems.push(`${f}: hardcoded node-version — use node-version-file: '.nvmrc' instead`);
    }
    // dtolnay/rust-toolchain must pin an exact toolchain. `@stable` is a
    // floating revision; `@master` is only acceptable when an explicit
    // `toolchain:` input is provided (e.g. `toolchain: '1.95.0'`).
    const actionRe = /uses:\s*dtolnay\/rust-toolchain@(stable|master)/g;
    let m;
    while ((m = actionRe.exec(content)) !== null) {
      const tail = content.slice(m.index, m.index + 400);
      if (m[1] === 'stable') {
        problems.push(`${f}: rust-toolchain@stable is a floating revision — pin an exact toolchain via src-tauri/rust-toolchain.toml + toolchain: input`);
      } else if (!/\btoolchain:\s*['"]?\d+\.\d+\.\d+/.test(tail) || /\btoolchain:\s*['"]?stable/.test(tail)) {
        problems.push(`${f}: rust-toolchain@master must specify an exact toolchain: '1.95.0' input`);
      }
    }
  }

  return problems;
}

// ---------------------------------------------------------------------------
// CLI parsing
// ---------------------------------------------------------------------------
function argValue(flag) {
  const hit = process.argv.slice(2).find((a) => a.startsWith(flag));
  return hit ? hit.slice(flag.length) : undefined;
}

const only = argValue('--step=');
const skipList = argValue('--skip=');
if (process.argv.includes('--list')) {
  for (const s of STEPS) process.stdout.write(`${s.id.padEnd(9)} ${s.name}\n`);
  process.exit(0);
}

let steps = STEPS;
if (only) {
  const ids = only.split(',').map((s) => s.trim()).filter(Boolean);
  const missing = ids.filter((id) => !STEPS.some((s) => s.id === id));
  if (missing.length) {
    console.error(`Unknown step(s): ${missing.join(', ')}`);
    process.exit(2);
  }
  steps = STEPS.filter((s) => ids.includes(s.id));
}
if (skipList) {
  const ids = skipList.split(',').map((s) => s.trim()).filter(Boolean);
  steps = steps.filter((s) => !ids.includes(s.id));
}
if (steps.length === 0) {
  console.error('No steps selected.');
  process.exit(2);
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------
function runStep(step) {
  if (step.fn) {
    try {
      const problems = step.fn();
      if (problems.length) {
        for (const p of problems) console.error(`  ✖ ${p}`);
        return { ok: false, code: 1, note: `problems: ${problems.length}` };
      }
      return { ok: true, code: 0 };
    } catch (err) {
      return { ok: false, code: 1, note: String(err) };
    }
  }
  const env = { ...process.env, ...(step.env || {}), FORCE_COLOR: '1' };
  // DEP0190 Node warning: shell:true concatenates args. We own every arg here
  // (static values above) and need a shell on Windows for .cmd shims, so mute
  // the deprecation just for this spawn.
  const prev = process.noDeprecation;
  process.noDeprecation = true;
  const res = spawnSync(step.cmd, step.args, {
    cwd: step.cwd ? path.join(ROOT, step.cwd) : ROOT,
    env,
    stdio: ['ignore', 'inherit', 'inherit'],
    shell: process.platform === 'win32',
  });
  process.noDeprecation = prev;
  if (res.error) return { ok: false, code: 1, note: res.error.message };
  return { ok: res.status === 0, code: res.status ?? res.signal ?? 1 };
}

let failed = null;
for (const [i, step] of steps.entries()) {
  process.stdout.write(`\n▸ (${i + 1}/${steps.length}) ${step.name} [${step.id}]\n`);
  const res = runStep(step);
  if (res.ok) {
    process.stdout.write('  ✓ passed\n');
  } else {
    process.stdout.write('  ✖ FAILED\n');
    if (res.note) process.stdout.write(`    ${res.note}\n`);
    failed = { step, ...res };
    break;
  }
}

if (failed) {
  console.error(`\n❌ Health check FAILED at "${failed.step.name}" [${failed.step.id}] (exit ${failed.code}).`);
  console.error(`   Reproduce locally with: node scripts/health-check.mjs --step=${failed.step.id}`);
  process.exit(failed.code || 1);
}

console.log(`\n✓ Health check passed — ${steps.length} check(s).`);

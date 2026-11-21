import { readFileSync, writeFileSync, readdirSync, statSync } from 'fs';
import { fileURLToPath } from 'url';
import { dirname, join, relative, sep } from 'path';

/**
 * check-error-catalog.mjs
 *
 * Gate for the central error catalog (docs/architecture/UNIVERSAL-CONTRACTS.md
 * section 2). Reads src-tauri/src/errors/catalog.rs and enforces:
 *   (a) every ErrorCode variant has exactly one CATALOG entry with an id and
 *       an i18n_key, and vice versa (no orphan rows, no orphan variants);
 *   (b) ids are unique and shaped `domain.snake_case`;
 *   (c) every `errors.<id>` key exists (non-empty) in en/pt/es/fr;
 *   (d) a ratchet over `Result<_, String>` occurrences in src-tauri/src:
 *       the count per file may only go down (scripts/error-catalog-allowlist.json).
 *
 * Usage:
 *   node scripts/check-error-catalog.mjs           # gate
 *   node scripts/check-error-catalog.mjs --update  # rewrite the allowlist
 *                                                  # after legitimate reductions
 */

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, '..');
const catalogPath = join(root, 'src-tauri', 'src', 'errors', 'catalog.rs');
const allowlistPath = join(root, 'scripts', 'error-catalog-allowlist.json');
const locales = ['en', 'pt', 'es', 'fr'];

const failures = [];
const notes = [];
const fail = (message) => failures.push(message);

// ---------------------------------------------------------------------------
// 1. Parse catalog.rs
// ---------------------------------------------------------------------------
let source;
try {
  source = readFileSync(catalogPath, 'utf8');
} catch (err) {
  console.error(`check-error-catalog: cannot read ${catalogPath}: ${err.message}`);
  process.exit(1);
}
// Normalize line endings: editors on Windows may save CRLF, which would
// otherwise defeat the $-anchored comment stripping below.
source = source.replace(/\r\n/g, '\n');

const stripLineComments = (text) =>
  text
    .split('\n')
    .map((line) => line.replace(/(^|\s)\/\/.*$/, '$1'))
    .join('\n');

const enumMatch = source.match(/pub enum ErrorCode\s*\{([\s\S]*?)\n\}/);
if (!enumMatch) {
  console.error('check-error-catalog: `pub enum ErrorCode { ... }` not found in catalog.rs');
  process.exit(1);
}
const enumVariants = [...stripLineComments(enumMatch[1]).matchAll(/\b([A-Z][A-Za-z0-9]*)\b/g)]
  .map((m) => m[1])
  .filter((name) => name !== 'ErrorCode');

const catalogMatch = source.match(/pub const CATALOG:\s*&\[ErrorSpec\]\s*=\s*&\[([\s\S]*?)\n\];/);
if (!catalogMatch) {
  console.error('check-error-catalog: `pub const CATALOG: &[ErrorSpec]` not found in catalog.rs');
  process.exit(1);
}
const entries = catalogMatch[1]
  .split('ErrorSpec {')
  .slice(1)
  .map((chunk) => {
    const code = chunk.match(/code:\s*(?:Self|ErrorCode)::(\w+)/)?.[1];
    const id = chunk.match(/id:\s*"([^"]+)"/)?.[1];
    const i18nKey = chunk.match(/i18n_key:\s*"([^"]+)"/)?.[1];
    return { code, id, i18nKey };
  });

if (entries.some((e) => !e.code || !e.id || !e.i18nKey)) {
  fail('CATALOG has a row missing code/id/i18n_key');
}

// (b) unique, domain.snake_case ids
{
  const seen = new Map();
  for (const entry of entries) {
    if (!entry.id) continue;
    if (seen.has(entry.id)) fail(`duplicate catalog id: ${entry.id}`);
    seen.set(entry.id, entry.code);
  }
  const idShape = /^[a-z][a-z0-9_]*\.[a-z][a-z0-9_]*$/;
  for (const entry of entries) {
    if (entry.id && !idShape.test(entry.id)) fail(`id is not domain.snake_case: ${entry.id}`);
    if (entry.i18nKey !== `errors.${entry.id}`) {
      fail(`i18n_key must be errors.<id>: ${entry.id} -> ${entry.i18nKey}`);
    }
  }
}

// (a) enum/catalog parity
{
  const variantSet = new Set(enumVariants);
  const entrySet = new Set(entries.map((e) => e.code).filter(Boolean));
  for (const variant of variantSet) {
    if (!entrySet.has(variant)) fail(`ErrorCode variant without CATALOG entry: ${variant}`);
  }
  for (const entry of entries) {
    if (entry.code && !variantSet.has(entry.code)) fail(`CATALOG entry without ErrorCode variant: ${entry.code}`);
  }
  notes.push(`catalog: ${variantSet.size} variants, ${entries.length} rows`);
}

// ---------------------------------------------------------------------------
// 2. Locales: every i18n_key must resolve to a non-empty leaf in all locales
// ---------------------------------------------------------------------------
const readLocale = (locale) => {
  try {
    return JSON.parse(readFileSync(join(root, 'src', 'i18n', `${locale}.json`), 'utf8'));
  } catch (err) {
    fail(`[${locale}] unreadable: ${err.message}`);
    return null;
  }
};

const lookupPath = (obj, key) => {
  let node = obj;
  for (const part of key.split('.')) {
    if (node === null || typeof node !== 'object' || !(part in node)) return undefined;
    node = node[part];
  }
  return node;
};

for (const locale of locales) {
  const data = readLocale(locale);
  if (!data) continue;
  for (const entry of entries) {
    const value = lookupPath(data, entry.i18nKey);
    if (typeof value !== 'string' || value.trim() === '') {
      fail(`[${locale}] missing or empty key: ${entry.i18nKey}`);
    }
  }
}

// ---------------------------------------------------------------------------
// 3. Ratchet: Result<_, String> occurrences under src-tauri/src
// ---------------------------------------------------------------------------
const countMatches = (rsPath) => {
  const text = readFileSync(rsPath, 'utf8');
  let count = 0;
  for (const line of text.split('\n')) {
    count += (line.match(/Result<[^>]+, String>/g) ?? []).length;
  }
  return count;
};

const walkRs = (dir, base) => {
  const out = [];
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    const rel = relative(base, full).split(sep).join('/');
    const stat = statSync(full);
    if (stat.isDirectory()) out.push(...walkRs(full, base));
    else if (rel.endsWith('.rs')) out.push(rel);
  }
  return out;
};

const srcDir = join(root, 'src-tauri', 'src');
const current = {};
for (const rel of walkRs(srcDir, srcDir)) {
  const count = countMatches(join(srcDir, rel));
  if (count > 0) current[`src-tauri/src/${rel}`] = count;
}
const currentTotal = Object.values(current).reduce((a, b) => a + b, 0);

let allowlist = { files: {} };
try {
  allowlist = JSON.parse(readFileSync(allowlistPath, 'utf8'));
} catch {
  fail(`cannot read allowlist ${allowlistPath}`);
}

if (failures.length === 0) {
  const allowed = allowlist.files ?? {};
  // (d) no NEW occurrences: neither a higher count on a known file nor a
  // file that was never in the allowlist
  for (const [file, count] of Object.entries(current)) {
    if (!(file in allowed)) {
      fail(`NEW Result<_, String> occurrences in ${file} (${count}); return AppError instead`);
    } else if (count > allowed[file]) {
      fail(
        `Result<_, String> ratchet exceeded in ${file}: ${count} > ${allowed[file]}; return AppError instead`,
      );
    } else if (count < allowed[file]) {
      notes.push(`ratchet: ${file} improved, ${allowed[file]} -> ${count} (run with --update to persist)`);
    }
  }
}

// ---------------------------------------------------------------------------
// 4. Output
// ---------------------------------------------------------------------------
if (process.argv.includes('--update')) {
  const files = {};
  for (const file of Object.keys(current).sort()) files[file] = current[file];
  const payload = {
    comment:
      'Ratchet over Result<_, String> occurrences in src-tauri/src (grep-equivalent, line-based regex /Result<[^>]+, String>/). Counts may only go down: fixed files are removed with `node scripts/check-error-catalog.mjs --update`. New occurrences fail the gate; new files with occurrences fail the gate.',
    pattern: 'Result<[^>]+, String>',
    total: currentTotal,
    files,
  };
  writeFileSync(allowlistPath, `${JSON.stringify(payload, null, 2)}\n`);
  console.log(`allowlist rewritten: ${currentTotal} occurrences across ${Object.keys(files).length} files`);
} else {
  console.log(`error catalog: ${entries.length} codes, ${currentTotal} legacy Result<_, String> occurrences (allowlisted: ${allowlist.total ?? 'n/a'})`);
  for (const note of notes) console.log(note);
  for (const message of failures) console.error(`FAIL: ${message}`);
  if (failures.length > 0) process.exit(1);
  console.log('check-error-catalog: OK');
}

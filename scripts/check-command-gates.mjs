#!/usr/bin/env node
/**
 * SEC-11 - Command-gate ratchet.
 *
 * Every `#[tauri::command]` handler in `src-tauri/src` is inventoried. A
 * handler is a VIOLATION when its body performs a PRIVILEGED MUTATION (a write
 * to BIOS / EC / Windows services / drivers / HKLM policy) DIRECTLY, i.e.
 * without either:
 *
 *   - routing through the elevated bridge (`run_elevated*`), or
 *   - an explicit validation gate (allowlist / env flag / range check).
 *
 * Why: the `hq_*` BIOS family (SEC-11) forwarded a raw `req: String` straight
 * to `HQWmiCommonInterface` WMI methods, in-process, with no elevation and no
 * allowlist. Any XSS in the webview could reorder the boot devices or reset
 * the BIOS. The rest of the codebase already does this correctly (EC/driver/
 * service/HKLM writes go through `elev_bridge::run_elevated`, and the raw EC
 * path is allowlist+env+range gated). This gate freezes that invariant: a NEW
 * privileged-mutation command that bypasses elevation and validation fails
 * here immediately.
 *
 * The privileged sinks are matched on the *lowercase function path* (e.g.
 * `hq_wmi::set_performance_mode`), never on the CamelCase request types, so a
 * command that only names a typed envelope is not a false positive.
 *
 * Ratchet: the baseline `scripts/command-gates-baseline.json` holds today's
 * accepted exceptions (privileged sinks that are legitimately validated rather
 * than elevated). It starts at ZERO violations - the hq_* family was the only
 * unvalidated in-process privileged writer and is now closed. Fixing a site
 * lowers the count (lower always passes); adding one raises it and fails.
 *
 * Usage:
 *   node scripts/check-command-gates.mjs            # gate (exit 1 on growth)
 *   node scripts/check-command-gates.mjs --report   # print the full inventory
 *   node scripts/check-command-gates.mjs --update   # regenerate the baseline
 */

import { readFileSync, writeFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, '..');
const SRC = path.join(ROOT, 'src-tauri', 'src');
const BASELINE = path.join(__dirname, 'command-gates-baseline.json');

const COMMAND_ATTR = '#[tauri::command]';

/**
 * Privileged mutation sinks. A command body matching any of these writes to
 * firmware or machine-wide state. Matched case-sensitively on the real call
 * path so typed envelopes (CamelCase) do not trip it.
 */
const SINKS = [
  { id: 'bios', re: /\bhq_wmi::(?:set_performance_mode|change_boot_option|load_default|load_default_key|s5_rtc_wake_enable|enable_pxe_boot|clear_key|set_oob_test_mode|clear_oob_test_mode|set_wifi_country_code|set_shipping_country_code)\s*\(/ },
  { id: 'ec-write', re: /\bwmi_ec::(?:wmi_write|set_[a-z0-9_]+)\s*\(/ },
  { id: 'eram-write', re: /\becram::write_ecram\s*\(/ },
  { id: 'service', re: /\bwin_service::(?:install|start|stop|delete|create|set_failure_actions|configure)\s*\(/ },
  { id: 'driver', re: /(?:\bpnputil\b|\badd_driver_package\b|\bdriver_install\s*\()/ },
  { id: 'hklm', re: /(?:RegistryHive::LocalMachine|HKEY_LOCAL_MACHINE|\bHKLM\\\w)/ },
];

/** Elevation markers: the handler delegates the write to the elevated helper. */
const ELEVATION = /\brun_elevated(?:_no_prompt|_bridge_only|_strict)?\s*\(/;

/**
 * Validation markers: an in-process privileged write that is nonetheless
 * guarded by an allowlist, env flag or range check. These are the legitimate
 * exceptions the baseline holds (e.g. `write_iot_hex`).
 */
const VALIDATION =
  /\b(?:is_known_safe_single_byte_write|raw_ecram_write_enabled|is_eram_range|is_known_ec_range|is_allowed_wmi_read|is_allowed_wmi_write|deny_unknown_fields|ALLOWED_COUNTRY_CODES|ALLOWED_SECRET_KEYS|from_args)\b/;

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
 * Split a file into per-command segments. For each `#[tauri::command]` we
 * locate the following `fn name`, then extract ONLY the signature and the
 * braced body via brace counting. This keeps helper functions defined between
 * two commands from being mis-attributed to the preceding handler. Files with
 * no `#[tauri::command]` (the elevated dispatcher `elevated.rs`, the service
 * binaries) contribute zero segments by construction, so their legitimate sink
 * calls are never scanned as commands.
 */
function commandSegments(text) {
  const segments = [];
  const attrIdx = [];
  let from = 0;
  while (true) {
    const i = text.indexOf(COMMAND_ATTR, from);
    if (i === -1) break;
    attrIdx.push(i);
    from = i + COMMAND_ATTR.length;
  }
  for (let k = 0; k < attrIdx.length; k++) {
    const start = attrIdx[k];
    const end = k + 1 < attrIdx.length ? attrIdx[k + 1] : text.length;
    const region = text.slice(start, end);
    const nameMatch = /\bfn\s+([A-Za-z0-9_]+)/.exec(region);
    if (!nameMatch) continue;
    const bodyStart = region.indexOf('{', nameMatch.index);
    if (bodyStart === -1) continue;
    // Count braces to find the body's matching close, ignoring the naive
    // string/comment edge cases (acceptable heuristic, as in the sibling gates).
    let depth = 0;
    let bodyEnd = -1;
    for (let j = bodyStart; j < region.length; j++) {
      const c = region[j];
      if (c === '{') depth++;
      else if (c === '}') {
        depth--;
        if (depth === 0) {
          bodyEnd = j;
          break;
        }
      }
    }
    const body = bodyEnd === -1 ? region.slice(bodyStart) : region.slice(bodyStart, bodyEnd + 1);
    // Include the signature (between the fn name and the body) so a typed
    // envelope in the parameter list is visible, but the sink scan targets the
    // call expressions in the body.
    segments.push({ name: nameMatch[1], signature: region.slice(0, bodyStart), body });
  }
  return segments;
}

/** Which sinks a segment body hits. */
function sinksIn(body) {
  const hit = [];
  for (const s of SINKS) {
    s.re.lastIndex = 0;
    if (s.re.test(body)) hit.push(s.id);
  }
  return hit;
}

/**
 * Inventory every command handler. Returns:
 *   violations: [{ file, name, sinks }]  privileged sink, no elevation/validation
 *   gated:      [{ file, name, sinks, via }]  privileged sink, properly gated
 */
function inventory() {
  const violations = [];
  const gated = [];
  for (const file of collectRustFiles(SRC)) {
    const rel = path.relative(ROOT, file).replaceAll('\\', '/');
    const text = readFileSync(file, 'utf8');
    if (!text.includes(COMMAND_ATTR)) continue;
    for (const seg of commandSegments(text)) {
      const haystack = `${seg.signature}\n${seg.body}`;
      const sinks = sinksIn(haystack);
      if (sinks.length === 0) continue;
      const elevated = ELEVATION.test(haystack);
      const validated = VALIDATION.test(haystack);
      if (elevated || validated) {
        gated.push({
          file: rel,
          name: seg.name,
          sinks,
          via: elevated ? 'elevation' : 'validation',
        });
      } else {
        violations.push({ file: rel, name: seg.name, sinks });
      }
    }
  }
  return { violations, gated };
}

const { violations, gated } = inventory();

// Key violations by "file#name" so the ratchet can compare per-site.
function keyOf(v) {
  return `${v.file}#${v.name}`;
}

if (argUpdate) {
  const sorted = violations
    .map((v) => ({ site: keyOf(v), sinks: v.sinks.sort() }))
    .sort((a, b) => a.site.localeCompare(b.site));
  writeFileSync(
    BASELINE,
    `${JSON.stringify(
      {
        comment:
          'Per-site inventory of privileged-mutation command handlers that bypass elevation AND validation. Ratchet baseline for scripts/check-command-gates.mjs; regenerate with --update. Expected to stay EMPTY: any entry is an unguarded BIOS/EC/service/driver/HKLM writer reachable from the webview.',
        total: sorted.length,
        sites: Object.fromEntries(sorted.map((s) => [s.site, s.sinks])),
      },
      null,
      2,
    )}\n`,
  );
  console.log(`baseline updated: ${sorted.length} ungated privileged-mutation site(s)`);
  if (argReport) {
    for (const s of sorted) console.log(`  - ${s.site}: ${s.sinks.join(',')}`);
  }
  process.exit(0);
}

let baseline = { total: 0, sites: {} };
try {
  baseline = JSON.parse(readFileSync(BASELINE, 'utf8'));
} catch {
  console.error(
    'baseline missing: run "node scripts/check-command-gates.mjs --update" first (expected an empty baseline).',
  );
  process.exit(1);
}

if (argReport) {
  console.log('Gated privileged-mutation command handlers:');
  for (const g of gated.sort((a, b) => a.name.localeCompare(b.name))) {
    console.log(`  [ok] ${g.file}#${g.name} [${g.sinks.join(',')}] via ${g.via}`);
  }
  console.log(`\nUngated violations (${violations.length}):`);
  for (const v of violations) {
    console.log(`  [!!] ${keyOf(v)}: ${v.sinks.join(',')}`);
  }
}

const baselineSites = baseline.sites ?? {};
const currentKeys = new Set(violations.map(keyOf));
const growth = [...currentKeys].filter((k) => !(k in baselineSites));
const improved = Object.keys(baselineSites).filter((k) => !currentKeys.has(k));

for (const k of improved) {
  console.log(`${k}: resolved (was an accepted violation) - lower is allowed`);
}

if (growth.length > 0) {
  console.error(`\ncommand-gate ratchet FAILED (${growth.length} new ungated site(s)):`);
  for (const k of growth) {
    const v = violations.find((x) => keyOf(x) === k);
    console.error(`  - ${k}: privileged sink [${v.sinks.join(',')}] with no elevation/validation`);
  }
  console.error(
    '\nRoute the write through elev_bridge::run_elevated, or gate it with an allowlist / env flag / range check (see commands/hardware.rs write_iot_hex and the hq_* typed envelopes in hw/hq_wmi.rs).',
  );
  process.exit(1);
}

console.log(
  `command-gate ratchet OK: ${gated.length} gated privileged-mutation handler(s), ${violations.length} ungated (baseline ${baseline.total}), no growth`,
);

#!/usr/bin/env node
/**
 * S65-003 — Release artifact contract: single source of truth for the
 * artifact names and URLs used by BOTH `scripts/publish-release.ps1`
 * (local publisher) and `.github/workflows/release.yml` (CI).
 *
 * Architecture (v2 decoupled installer, see docs/installer-decoupling.md):
 *   * MiControl_Setup.exe           — the STATIC setup shell (stable hash).
 *   * MiControl_<ver>_payload.zip   — the VERSIONED, minisign-signed payload.
 *   * latest.json                   — Tauri updater manifest → payload URL.
 *
 * The legacy NSIS installer (`MiControl_<ver>_x64-setup.exe`) is still
 * built and uploaded as a manual fallback, but it must NEVER be the
 * updater artifact: the updater downloads the payload and hands it to the
 * installed shell.
 *
 * Usage:
 *   node scripts/release-contract.mjs check  <version> <dir-with-assets>
 *       → verifies that every required asset exists in <dir> and that
 *         latest.json (when present) points at the payload URL, not the
 *         NSIS URL. Exits 1 with a clear report on any violation.
 *
 *   node scripts/release-contract.mjs names  <version>
 *       → prints the canonical asset names + GitHub URLs for a version.
 *         Both the local publisher and the CI workflow must use these.
 *
 *   node scripts/release-contract.mjs dry-run <version> <payload-zip> <sig>
 *       → offline validation of the staged asset set (zip + sig + names)
 *         WITHOUT calling `gh release` or pushing any tag. This is the
 *         "offline release test" required by S65-003: it proves the
 *         staged set is complete and well-formed before any publication.
 */
import { readFileSync, existsSync, openSync, readSync, closeSync } from 'node:fs';
import path from 'node:path';

const PRODUCT = 'MiControl';
const REPO = 'arcane-D7/micontrol';

// ── Canonical names (single source of truth) ───────────────────────────────

export function contractNames(version) {
  const payload = `${PRODUCT}_${version}_payload.zip`;
  const payloadSig = `${payload}.sig`;
  const shell = `${PRODUCT}_Setup.exe`;
  const nsis = `${PRODUCT}_${version}_x64-setup.exe`; // manual fallback only
  const base = `https://github.com/${REPO}/releases/latest/download`;
  return {
    version,
    assets: {
      payload,
      payloadSig,
      shell,
      nsis,
      latestJson: 'latest.json',
    },
    urls: {
      payload: `${base}/${payload}`,
      payloadSig: `${base}/${payloadSig}`,
      shell: `${base}/${shell}`,
      nsis: `${base}/${nsis}`, // never the updater artifact
      latestJson: `${base}/latest.json`,
    },
  };
}

// ── S65-004: Required vs optional payload contents ──────────────────────────
// Single source of truth for payload validation. publish-release.ps1 checks
// REQUIRED entries BEFORE zipping; the shell preflight in
// src-tauri/shell/src/payload.rs enforces the same set after extracting.
// Keep these two lists in sync when adding new payload entries.

/** Files that MUST be present for a payload to be considered complete. */
export const REQUIRED_PAYLOAD_ENTRIES = [
  'micontrol.exe',
  'ecram_service.exe',
  'micontrol_bridge.exe',
];

/** Files that MAY be absent; their omission is logged but never fatal. */
export const OPTIONAL_PAYLOAD_ENTRIES = [
  'micontrol_face_svc.exe',
  'micontrol_facecp.dll',
  'WebView2Loader.dll',
  'drivers',
  'resources',
  'payload.json',
];

/**
 * Validate a payload zip against the contract.
 *
 * @param {import('node:fs').FileHandle} - not used; callers read the zip
 *        themselves. This function takes the list of top-level entry names
 *        found in the archive.
 * @param {string[]} entries - top-level names inside the zip.
 * @returns {{ok: boolean, missingRequired: string[], missingOptional: string[]}}
 */
export function validatePayloadEntries(entries) {
  const set = new Set(entries);
  const missingRequired = REQUIRED_PAYLOAD_ENTRIES.filter((e) => !set.has(e));
  const missingOptional = OPTIONAL_PAYLOAD_ENTRIES.filter((e) => !set.has(e));
  return {
    ok: missingRequired.length === 0,
    missingRequired,
    missingOptional,
  };
}

// ── Subcommands ─────────────────────────────────────────────────────────────

function cmdNames(version) {
  const c = contractNames(version);
  console.log(JSON.stringify(c, null, 2));
}

/**
 * Verify that a staged directory contains every REQUIRED asset and that
 * latest.json (when present) targets the payload — not the NSIS installer.
 *
 * Required: payload, payloadSig, shell, latest.json
 * Optional: nsis (manual fallback; presence never fails the check)
 *
 * @returns {{ok: boolean, missing: string[], violations: string[]}}
 */
export function checkStagedAssets(stagedDir, version) {
  const c = contractNames(version);
  const missing = [];
  const violations = [];

  const required = [
    c.assets.payload,
    c.assets.payloadSig,
    c.assets.shell,
    c.assets.latestJson,
  ];
  for (const name of required) {
    if (!existsSync(path.join(stagedDir, name))) {
      missing.push(name);
    }
  }

  // Validate latest.json content when present.
  const latestPath = path.join(stagedDir, c.assets.latestJson);
  if (existsSync(latestPath)) {
    let manifest;
    try {
      manifest = JSON.parse(readFileSync(latestPath, 'utf8'));
    } catch (e) {
      violations.push(`latest.json is not valid JSON: ${e.message}`);
    }
    if (manifest) {
      const url =
        manifest.platforms?.['windows-x86_64']?.url ?? '';
      if (url.includes('_x64-setup.exe')) {
        violations.push(
          `latest.json targets the NSIS installer (${url}) — it must target ` +
            `the payload (${c.urls.payload}). The NSIS setup is a manual ` +
            `fallback and must never be the updater artifact.`,
        );
      }
      if (url && !url.endsWith(c.assets.payload)) {
        violations.push(
          `latest.json url (${url}) does not end with the expected payload ` +
            `name (${c.assets.payload}).`,
        );
      }
      if (manifest.version && manifest.version !== version) {
        violations.push(
          `latest.json version (${manifest.version}) does not match ` +
            `the staged version (${version}).`,
        );
      }
      const sig = manifest.platforms?.['windows-x86_64']?.signature;
      if (!sig || sig.length < 32) {
        violations.push(
          'latest.json is missing a valid signature in ' +
            'platforms.windows-x86_64.signature.',
        );
      }
    }
  }

  return { ok: missing.length === 0 && violations.length === 0, missing, violations };
}

function cmdCheck(version, stagedDir) {
  const result = checkStagedAssets(stagedDir, version);
  if (result.missing.length) {
    console.error(`Missing required assets in ${stagedDir}:`);
    for (const m of result.missing) console.error(`  ✖ ${m}`);
  }
  if (result.violations.length) {
    console.error('Contract violations:');
    for (const v of result.violations) console.error(`  ✖ ${v}`);
  }
  if (result.ok) {
    console.log(`✓ Staged asset set for ${version} is complete and compliant.`);
    process.exit(0);
  }
  process.exit(1);
}

/**
 * Offline validation of a payload zip + signature pair WITHOUT touching
 * GitHub. Checks:
 *   1. The zip exists and is non-empty.
 *   2. The .sig exists and is non-empty.
 *   3. Both names match the canonical contract for the version.
 *   4. (Best-effort) The zip is a valid ZIP archive (PK header).
 *
 * This is the "offline/dry-run release test" required by S65-003.
 *
 * @param {string} version
 * @param {string} payloadZip - path to MiControl_<ver>_payload.zip
 * @param {string} payloadSig - path to MiControl_<ver>_payload.zip.sig
 */
function cmdDryRun(version, payloadZip, payloadSig) {
  const c = contractNames(version);
  const problems = [];

  // 1. Existence + non-empty.
  for (const [label, p] of [['payload zip', payloadZip], ['signature', payloadSig]]) {
    if (!existsSync(p)) {
      problems.push(`${label} not found: ${p}`);
      continue;
    }
    const stat = readFileSync(p);
    if (stat.length === 0) {
      problems.push(`${label} is empty: ${p}`);
    }
  }

  // 2. Name match.
  const zipBasename = path.basename(payloadZip);
  const sigBasename = path.basename(payloadSig);
  if (zipBasename !== c.assets.payload) {
    problems.push(
      `payload zip name (${zipBasename}) does not match contract (${c.assets.payload}).`,
    );
  }
  if (sigBasename !== c.assets.payloadSig) {
    problems.push(
      `signature name (${sigBasename}) does not match contract (${c.assets.payloadSig}).`,
    );
  }

  // 3. PK header check (best-effort, no zip dependency needed).
  if (existsSync(payloadZip)) {
    const header = Buffer.alloc(4);
    try {
      // ESM (.mjs) has no `require` — the fs functions are already imported
      // at the top of this module (openSync/readSync/closeSync).
      const fd = openSync(payloadZip, 'r');
      readSync(fd, header, 0, 4, 0);
      closeSync(fd);
      if (header[0] !== 0x50 || header[1] !== 0x4b) {
        problems.push(`${payloadZip} is not a valid ZIP archive (bad PK header).`);
      }
    } catch (e) {
      problems.push(`could not read ${payloadZip} header: ${e.message}`);
    }
  }

  // 4. Signature file should contain a minisign signature block.
  //    `tauri signer sign` writes the signature BASE64-ENCODED (the format the
  //    Tauri updater consumes — same as the NSIS bundle .sig); a raw minisign
  //    `sign` tool writes plain text. Both are valid: accept either.
  if (existsSync(payloadSig)) {
    let sigText = readFileSync(payloadSig, 'utf8').trim();
    // Base64 form: decode once and re-check the prefix.
    if (!sigText.startsWith('untrusted comment:') && !sigText.startsWith('threshold=')) {
      try {
        const decoded = Buffer.from(sigText, 'base64').toString('utf8');
        if (decoded.startsWith('untrusted comment:') || decoded.startsWith('threshold=')) {
          sigText = decoded;
        }
      } catch {
        // Not base64 either — fall through to the problem report below.
      }
    }
    if (!sigText.startsWith('untrusted comment:') && !sigText.startsWith('threshold=')) {
      problems.push(
        `${payloadSig} does not look like a minisign signature file ` +
          `(expected 'untrusted comment:' or 'threshold=' prefix, plain or base64).`,
      );
    }
  }

  if (problems.length) {
    console.error('Dry-run release validation FAILED:');
    for (const p of problems) console.error(`  ✖ ${p}`);
    process.exit(1);
  }
  console.log(`✓ Dry-run OK: ${c.assets.payload} + ${c.assets.payloadSig} are well-formed.`);
  console.log(`  Updater URL: ${c.urls.payload}`);
  process.exit(0);
}

// ── CLI entry ────────────────────────────────────────────────────────────────

const [cmd, ...args] = process.argv.slice(2);
switch (cmd) {
  case 'names':
    cmdNames(args[0] ?? process.env.MICONTROL_VERSION ?? '0.0.0');
    break;
  case 'check':
    cmdCheck(args[0] ?? process.env.MICONTROL_VERSION ?? '0.0.0', args[1] ?? '.');
    break;
  case 'dry-run':
    cmdDryRun(args[0] ?? '0.0.0', args[1], args[2]);
    break;
  default:
    console.error(
      'Usage:\n' +
        '  release-contract.mjs names    <version>\n' +
        '  release-contract.mjs check    <version> <staged-dir>\n' +
        '  release-contract.mjs dry-run  <version> <payload.zip> <payload.sig>',
    );
    process.exit(2);
}

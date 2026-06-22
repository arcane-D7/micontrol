#!/usr/bin/env node
/**
 * Version Sync Script — S7-004
 *
 * Reads the version from package.json (single source of truth) and
 * propagates it to src-tauri/Cargo.toml and src-tauri/tauri.conf.json.
 *
 * Usage:
 *   node scripts/sync-version.js          # Sync from package.json
 *   node scripts/sync-version.js 1.2.3    # Bump to 1.2.3 and sync
 */

const fs = require('fs');
const path = require('path');

const ROOT = path.resolve(__dirname, '..');
const PACKAGE_JSON = path.join(ROOT, 'package.json');
const CARGO_TOML = path.join(ROOT, 'src-tauri', 'Cargo.toml');
const TAURI_CONF = path.join(ROOT, 'src-tauri', 'tauri.conf.json');

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, 'utf8'));
}

function writeJson(filePath, data) {
  fs.writeFileSync(filePath, JSON.stringify(data, null, 2) + '\n', 'utf8');
}

function readCargoVersion(filePath) {
  const content = fs.readFileSync(filePath, 'utf8');
  const match = content.match(/^version\s*=\s*"([^"]+)"/m);
  return match ? match[1] : null;
}

function writeCargoVersion(filePath, version) {
  let content = fs.readFileSync(filePath, 'utf8');
  content = content.replace(
    /^version\s*=\s*"[^"]+"/m,
    `version = "${version}"`
  );
  fs.writeFileSync(filePath, content, 'utf8');
}

function sync(newVersion) {
  // Read source version
  const pkg = readJson(PACKAGE_JSON);
  const sourceVersion = newVersion || pkg.version;

  if (newVersion) {
    pkg.version = newVersion;
    writeJson(PACKAGE_JSON, pkg);
    console.log(`✓ package.json: ${newVersion}`);
  } else {
    console.log(`Source: package.json version = ${sourceVersion}`);
  }

  // Sync to Cargo.toml
  const cargoVersion = readCargoVersion(CARGO_TOML);
  if (cargoVersion !== sourceVersion) {
    writeCargoVersion(CARGO_TOML, sourceVersion);
    console.log(`✓ Cargo.toml: ${cargoVersion} → ${sourceVersion}`);
  } else {
    console.log(`✓ Cargo.toml: already ${cargoVersion}`);
  }

  // Sync to tauri.conf.json
  const tauriConf = readJson(TAURI_CONF);
  if (tauriConf.version !== sourceVersion) {
    tauriConf.version = sourceVersion;
    writeJson(TAURI_CONF, tauriConf);
    console.log(`✓ tauri.conf.json: ${tauriConf.version} → ${sourceVersion}`);
  } else {
    console.log(`✓ tauri.conf.json: already ${sourceVersion}`);
  }

  console.log(`\nAll versions synced to ${sourceVersion}`);
}

function check() {
  const pkg = readJson(PACKAGE_JSON);
  const cargoVersion = readCargoVersion(CARGO_TOML);
  const tauriConf = readJson(TAURI_CONF);

  const versions = {
    'package.json': pkg.version,
    'Cargo.toml': cargoVersion,
    'tauri.conf.json': tauriConf.version,
  };

  const allMatch = Object.values(versions).every(v => v === pkg.version);

  if (allMatch) {
    console.log(`✓ All versions agree: ${pkg.version}`);
    process.exit(0);
  } else {
    console.error('✗ Version mismatch detected:');
    for (const [file, version] of Object.entries(versions)) {
      const status = version === pkg.version ? '✓' : '✗';
      console.error(`  ${status} ${file}: ${version}`);
    }
    console.error('\nRun `npm run version:sync` to fix.');
    process.exit(1);
  }
}

// CLI
const arg = process.argv[2];
if (arg === '--check') {
  check();
} else if (arg) {
  sync(arg);
} else {
  sync();
}
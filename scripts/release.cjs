#!/usr/bin/env node
/**
 * Release Script
 *
 * Bumps version, syncs all config files, commits, tags, and pushes.
 * The GitHub Actions release.yml workflow triggers on the tag push
 * and builds + signs + publishes the installer automatically.
 *
 * Usage:
 *   node scripts/release.cjs patch    # 1.0.0 → 1.0.1
 *   node scripts/release.cjs minor    # 1.0.0 → 1.1.0
 *   node scripts/release.cjs major    # 1.0.0 → 2.0.0
 *   node scripts/release.cjs 1.2.3    # explicit version
 */

const { execSync } = require('child_process');
const path = require('path');

const ROOT = path.resolve(__dirname, '..');

function run(cmd, opts = {}) {
  console.log(`  $ ${cmd}`);
  return execSync(cmd, { cwd: ROOT, stdio: 'pipe', encoding: 'utf8', ...opts }).trim();
}

function runInherit(cmd) {
  console.log(`  $ ${cmd}`);
  execSync(cmd, { cwd: ROOT, stdio: 'inherit' });
}

function bumpVersion(current, type) {
  const parts = current.split('.').map(Number);
  if (parts.length !== 3 || parts.some(isNaN)) {
    console.error(`✗ Invalid version format: ${current}`);
    process.exit(1);
  }
  let [major, minor, patch] = parts;
  switch (type) {
    case 'major':
      major++;
      minor = 0;
      patch = 0;
      break;
    case 'minor':
      minor++;
      patch = 0;
      break;
    case 'patch':
      patch++;
      break;
    default:
      // Assume explicit version (e.g. "1.2.3")
      if (!/^\d+\.\d+\.\d+$/.test(type)) {
        console.error(`✗ Invalid bump type or version: ${type}`);
        console.error('  Use: patch, minor, major, or an explicit version like 1.2.3');
        process.exit(1);
      }
      return type;
  }
  return `${major}.${minor}.${patch}`;
}

function main() {
  const type = process.argv[2];

  if (!type) {
    console.error('Usage: node scripts/release.cjs <patch|minor|major|version>');
    console.error('  patch  — 1.0.0 → 1.0.1');
    console.error('  minor  — 1.0.0 → 1.1.0');
    console.error('  major  — 1.0.0 → 2.0.0');
    console.error('  1.2.3  — explicit version');
    process.exit(1);
  }

  // Read current version
  const pkg = require(path.join(ROOT, 'package.json'));
  const currentVersion = pkg.version;
  const newVersion = bumpVersion(currentVersion, type);

  console.log(`\n🚀 MiControl Release\n`);
  console.log(`  Current version: ${currentVersion}`);
  console.log(`  New version:     ${newVersion}\n`);

  // Check working tree is clean
  const status = run('git status --porcelain');
  if (status) {
    console.error('✗ Working tree is not clean. Commit or stash your changes first.');
    console.error(status);
    process.exit(1);
  }

  // Check we're on main branch
  const branch = run('git rev-parse --abbrev-ref HEAD');
  if (branch !== 'main') {
    console.error(`✗ Must be on 'main' branch (currently on '${branch}').`);
    process.exit(1);
  }

  // Check we're up to date with remote
  run('git fetch origin main');
  const localHead = run('git rev-parse HEAD');
  const remoteHead = run('git rev-parse origin/main');
  if (localHead !== remoteHead) {
    console.error('✗ Local main is not in sync with origin/main.');
    console.error('  Pull or push first.');
    process.exit(1);
  }

  // Bump version (sync-version.cjs updates package.json, Cargo.toml, tauri.conf.json)
  console.log('📝 Bumping version...');
  runInherit(`node scripts/sync-version.cjs ${newVersion}`);

  // Commit the version bump
  console.log('\n📦 Committing version bump...');
  run('git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json');
  run(`git commit -m "chore(release): v${newVersion}"`);

  // Create and push tag
  const tagName = `v${newVersion}`;
  console.log(`\n🏷️  Creating tag ${tagName}...`);
  run(`git tag -a ${tagName} -m "Release ${tagName}"`);

  // Push commit + tag
  console.log('\n📤 Pushing to origin...');
  runInherit('git push origin main');
  runInherit(`git push origin ${tagName}`);

  console.log(`\n✅ Done! Release v${newVersion} triggered.`);
  console.log(`   GitHub Actions will build and publish the installer.`);
  console.log(`   https://github.com/arcane-D7/micontrol/actions`);
  console.log(`   https://github.com/arcane-D7/micontrol/releases`);
  console.log('');
}

main();

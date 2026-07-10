#!/usr/bin/env node
/**
 * Clean build artifacts for micontrol (Tauri + Vite).
 *
 * Removes:
 *   - src-tauri/target/debug/ (keeps release/ for faster rebuilds)
 *   - dist/ and dist-landing/
 *   - coverage/
 *   - .vite/ cache
 */

import { readdirSync, rmSync, existsSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = fileURLToPath(new URL('.', import.meta.url));
const projectRoot = resolve(__dirname, '..');

function dirSize(dirPath) {
  let total = 0;
  try {
    const entries = readdirSync(dirPath, { withFileTypes: true });
    for (const entry of entries) {
      const fullPath = join(dirPath, entry.name);
      if (entry.isDirectory()) {
        total += dirSize(fullPath);
      } else {
        total += statSync(fullPath).size;
      }
    }
  } catch {
    // ignore
  }
  return total;
}

function removeDir(dirPath, label) {
  if (!existsSync(dirPath)) return;
  const sizeMB = (dirSize(dirPath) / 1024 / 1024).toFixed(1);
  rmSync(dirPath, { recursive: true, force: true });
  console.log(`  🗑️  ${label}: ${sizeMB} MB removed`);
}

console.log('🧹 Cleaning micontrol build artifacts...');

// Vite/TS build output
removeDir(join(projectRoot, 'dist'), 'dist');
removeDir(join(projectRoot, 'dist-landing'), 'dist-landing');
removeDir(join(projectRoot, 'coverage'), 'coverage');
removeDir(join(projectRoot, '.vite'), '.vite cache');

// Rust target/debug (keep release)
const debugDir = join(projectRoot, 'src-tauri', 'target', 'debug');
if (existsSync(debugDir)) {
  const sizeGB = (dirSize(debugDir) / 1024 / 1024 / 1024).toFixed(2);
  rmSync(debugDir, { recursive: true, force: true });
  console.log(`  🗑️  src-tauri/target/debug: ${sizeGB} GB removed`);
}

console.log('✅ Cleanup complete.\n');

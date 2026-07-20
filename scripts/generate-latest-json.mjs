// Generates latest.json for Tauri v2 updater
// Usage: node scripts/generate-latest-json.mjs <version> <sig-file> <installer-url> <output-path>
import { readFileSync, writeFileSync } from 'fs';

const [,, version, sigPath, installerUrl, outputPath] = process.argv;

if (!version || !sigPath || !installerUrl || !outputPath) {
  console.error('Usage: node scripts/generate-latest-json.mjs <version> <sig-file> <installer-url> <output-path>');
  process.exit(1);
}

const signature = readFileSync(sigPath, 'utf-8').trim();
const pubDate = new Date().toISOString();

const manifest = {
  version,
  notes: `MiControl v${version}`,
  pub_date: pubDate,
  platforms: {
    'windows-x86_64': {
      signature,
      url: installerUrl,
    },
  },
};

writeFileSync(outputPath, JSON.stringify(manifest, null, 2));
console.log(`Generated ${outputPath}`);
console.log(JSON.stringify(manifest, null, 2));

#!/usr/bin/env node
/**
 * Extract a version section from CHANGELOG.md for use as a GitHub Release body.
 *
 * Usage:
 *   node scripts/extract-changelog.mjs <version> [outputFile]
 *
 *   <version>     Version to extract, WITHOUT the leading "v" (e.g. "0.1.17").
 *                 Pass "Unreleased" to extract the top [Unreleased] section.
 *   [outputFile]  Optional path to write the extracted section to.
 *                 If omitted, the section is written to stdout.
 *
 * Example (used by release.yml):
 *   node scripts/extract-changelog.mjs "$version" "$RUNNER_TEMP/release-body.md"
 *
 * The extracted section matches the section header format used in
 * CHANGELOG.md, e.g. "## [0.1.17] - 2026-08-08". The lines right after the
 * version header up to (but not including) the NEXT "## " header are emitted.
 * The leading "## [..]" header line is kept so the release body reads well.
 */

import { readFileSync, writeFileSync } from 'fs';
import path from 'path';

const ROOT = path.resolve(import.meta.dirname, '..');
const CHANGELOG = path.join(ROOT, 'CHANGELOG.md');

function main() {
  const versionArg = process.argv[2];
  const output = process.argv[3];

  if (!versionArg) {
    console.error(
      'Usage: node scripts/extract-changelog.mjs <version|Unreleased> [outputFile]'
    );
    process.exit(1);
  }

  const version = versionArg.replace(/^v/, '');
  const content = readFileSync(CHANGELOG, 'utf8');
  const lines = content.split(/\r?\n/);

  // Find the section header for the requested version. The header format is:
  //   ## [0.1.17] - 2026-08-08     (or)    ## [Unreleased]
  const headerLineIdx = lines.findIndex((line) => {
    const m = line.match(/^##\s+\[([^\]]+)\]/);
    if (!m) return false;
    const headerVersion = m[1].trim();
    return (
      headerVersion === version ||
      headerVersion.replace(/^v/, '') === version
    );
  });

  if (headerLineIdx === -1) {
    console.error(
      `✗ Section "[${version}]" not found in ${CHANGELOG}. ` +
        'Checked for header "## [<version>]".'
    );
    process.exit(1);
  }

  // Collect lines until the next top-level "## " section header.
  const sectionLines = [lines[headerLineIdx]];
  for (let i = headerLineIdx + 1; i < lines.length; i++) {
    if (/^##\s/.test(lines[i])) break;
    sectionLines.push(lines[i]);
  }
  // Trim trailing blank lines.
  while (sectionLines.length && sectionLines[sectionLines.length - 1].trim() === '') {
    sectionLines.pop();
  }

  const section = sectionLines.join('\n') + '\n';

  if (output) {
    writeFileSync(output, section, 'utf8');
    console.log(`✓ Extracted "[${version}]" section (${sectionLines.length} lines) → ${output}`);
  } else {
    process.stdout.write(section);
  }
}

main();

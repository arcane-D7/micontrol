/**
 * S65-001 regression test: the S63 tray theme-specificity fix.
 *
 * Commit `353b388` over-qualified the tray selector so that the opaque
 * surface tokens win over the `prefers-color-scheme` media query:
 *
 *   html:root.tray-window:not([data-theme='light'])
 *     = (0,3,1) — beats `:root:not([data-theme=light])` (0,2,0).
 *
 * Commit `4386c5e` later reverted it back to `.tray-window` (0,1,0),
 * which loses to the media-query rule and re-introduces ghosted/tran-
 * sparent surfaces when the OS is in dark mode.
 *
 * This test pins the built CSS so the over-qualified selector is present
 * and the bare `.tray-window` rule is NOT present. A regression that
 * drops the over-qualification will fail this test immediately.
 *
 * Run via: `pnpm run health-check` → frontend build + vitest.
 */
import { describe, it, expect, beforeAll } from 'vitest';
import { readFileSync, existsSync, readdirSync, statSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

// `import.meta.url` points to this file at src/__tests__/TrayThemeRegression.test.ts,
// so the project root is two levels up.
const here = dirname(fileURLToPath(import.meta.url));
const DIST_ASSETS = resolve(here, '../../dist/assets');

function findBuiltCss(): string | null {
  try {
    const files = readdirSync(DIST_ASSETS).filter((f) => /^index-.*\.css$/.test(f));
    if (files.length === 0) return null;
    const withMtime = files.map((f) => ({ f, m: statSync(resolve(DIST_ASSETS, f)).mtimeMs }));
    withMtime.sort((a, b) => b.m - a.m);
    return resolve(DIST_ASSETS, withMtime[0].f);
  } catch {
    return null;
  }
}

describe('S65-001: S63 tray theme-specificity regression', () => {
  let css: string | null = null;

  beforeAll(() => {
    const file = findBuiltCss();
    if (!file || !existsSync(file)) {
      // Build hasn't run yet — skip gracefully. The health-check pipeline
      // always runs the build before tests, so this only triggers when
      // running vitest standalone without a build step.
      console.warn(
        '[TrayThemeRegression] dist/ build not found — skipping. ' +
          'Run `pnpm run build` first, or run the full health-check.',
      );
      return;
    }
    css = readFileSync(file, 'utf8');
  });

  it('the over-qualified tray selector is present in built CSS', () => {
    if (!css) return; // skipped
    // The exact selector from commit 353b388.
    const expected = "html:root.tray-window:not([data-theme='light'])";
    // Minified CSS strips quotes around the attribute value; check both forms.
    const minified = 'html:root.tray-window:not([data-theme=light])';
    const found = css.includes(expected) || css.includes(minified);
    expect(
      found,
      'Over-qualified tray selector not found in built CSS. ' +
        'The S63 specificity fix (commit 353b388) was reverted. ' +
        'Restore `html:root.tray-window:not([data-theme=light])` in src/styles/globals.css.',
    ).toBe(true);
  });

  it('the bare .tray-window selector is NOT used for surface overrides', () => {
    if (!css) return; // skipped
    // The reverted form (commit 4386c5e) used `.tray-window` directly.
    // After the fix, the surface-token overrides must be on the over-qualified
    // selector. A bare `.tray-window {` opening brace with `--surface` or
    // `--blur` inside would indicate the regression is back.
    const bareRuleRe = /\.tray-window\s*\{[^}]*--(surface|blur)[^}]*\}/g;
    const matches = css.match(bareRuleRe);
    expect(
      matches,
      `Bare .tray-window rule still sets surface/blur tokens: ${matches?.[0] ?? ''}. ` +
        'Move these to html:root.tray-window:not([data-theme=light]) instead.',
    ).toBeNull();
  });

  it('the transparent WebView background is preserved on the over-qualified rule', () => {
    if (!css) return; // skipped
    const expectedSelector = "html:root.tray-window:not([data-theme='light'])";
    const minifiedSelector = 'html:root.tray-window:not([data-theme=light])';
    const found = css.includes(expectedSelector) || css.includes(minifiedSelector);
    if (!found) return; // first test will catch the missing selector
    // Find the rule block and verify it contains the transparent background.
    const idx =
      css.indexOf(expectedSelector) !== -1
        ? css.indexOf(expectedSelector)
        : css.indexOf(minifiedSelector);
    const blockEnd = css.indexOf('}', idx);
    const block = css.slice(idx, blockEnd + 1);
    expect(block, 'Over-qualified tray rule should set background: transparent').toMatch(
      /background:\s*transparent/,
    );
  });
});

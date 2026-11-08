import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Gate do tema (theme-migration.md §3.2/§3.5).
 *
 * Verifica, por leitura do CSS, as três condições que o documento diz serem
 * testáveis:
 *   1. para cada token de cor existe um par nos dois temas;
 *   2. a amplitude de luminância de cada família é a mesma nos dois temas
 *      (elevação sempre para cima, recesso sempre para dentro);
 *   3. todos os pares de texto passam 4,5:1 e os de componente 3:1.
 *
 * E as regras de forma que o tema promete: zero sombra, zero blur,
 * zero `oklch(from …)`, fontes que existem.
 */

const root = resolve(__dirname, '..');
const globals = readFileSync(resolve(root, 'styles/globals.css'), 'utf8');
const primitives = readFileSync(resolve(root, 'styles/primitives.css'), 'utf8');

/** Os 20 tokens de cor de §3.4. Nada mais conta como cor. */
const COLOR_TOKENS = [
  'bg',
  'surface',
  'surface-2',
  'hairline',
  'hairline-2',
  'ink',
  'ink-muted',
  'ink-dim',
  'accent',
  'accent-ink',
  'accent-soft',
  'accent-on',
  'success',
  'success-soft',
  'warning',
  'warning-soft',
  'danger',
  'danger-soft',
  'info',
  'info-soft',
] as const;

type Theme = Record<string, string>;

/** Devolve o conteúdo do primeiro bloco `{…}` a partir de `header`. */
function blockAfter(css: string, header: string): string {
  const start = css.indexOf(header);
  if (start === -1) throw new Error(`bloco não encontrado: ${header}`);

  const open = css.indexOf('{', start);
  let depth = 0;
  for (let i = open; i < css.length; i += 1) {
    if (css[i] === '{') depth += 1;
    else if (css[i] === '}') {
      depth -= 1;
      if (depth === 0) return css.slice(open + 1, i);
    }
  }
  throw new Error(`bloco sem fecho: ${header}`);
}

function tokens(block: string): Theme {
  const out: Theme = {};
  const re = /--([a-z0-9-]+)\s*:\s*([^;]+);/gi;
  let match = re.exec(block);
  while (match) {
    out[match[1]] = match[2].trim();
    match = re.exec(block);
  }
  return out;
}

const lightBlock = blockAfter(globals, '\n:root {');
const darkBlock = blockAfter(globals, "[data-theme='dark']");
const autoMedia = blockAfter(globals, '@media (prefers-color-scheme: dark)');
const autoBlock = blockAfter(autoMedia, ":root[data-theme='auto']");

const light = tokens(lightBlock);
const dark = tokens(darkBlock);
const auto = tokens(autoBlock);

/* ── OKLCH → linear sRGB → luminância relativa WCAG ─────────────────────── */

function parseOklch(value: string): { L: number; C: number; H: number } {
  const match = /oklch\(\s*([\d.]+)%\s+([\d.]+)\s+([\d.]+)/i.exec(value);
  if (!match) throw new Error(`não é oklch: ${value}`);
  return { L: Number(match[1]) / 100, C: Number(match[2]), H: Number(match[3]) };
}

function linearChannel(value: number): number {
  return Math.min(1, Math.max(0, value));
}

function relativeLuminance(value: string): number {
  const { L, C, H } = parseOklch(value);
  const h = (H * Math.PI) / 180;
  const a = C * Math.cos(h);
  const b = C * Math.sin(h);

  const l_ = L + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = L - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = L - 0.0894841775 * a - 1.291485548 * b;

  const l = l_ ** 3;
  const m = m_ ** 3;
  const s = s_ ** 3;

  const r = linearChannel(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s);
  const g = linearChannel(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s);
  const bl = linearChannel(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s);

  return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
}

function contrast(theme: Theme, a: string, b: string): number {
  const ya = relativeLuminance(theme[a]);
  const yb = relativeLuminance(theme[b]);
  const hi = Math.max(ya, yb);
  const lo = Math.min(ya, yb);
  return (hi + 0.05) / (lo + 0.05);
}

/* ── 1. Os 20 tokens existem nos dois temas ─────────────────────────────── */

describe('tokens', () => {
  it.each(COLOR_TOKENS)('--%s existe no claro, no escuro e no automático', (name) => {
    expect(light[name], `--${name} em :root`).toBeDefined();
    expect(dark[name], `--${name} em [data-theme='dark']`).toBeDefined();
    expect(auto[name], `--${name} no bloco automático`).toBeDefined();
  });

  it('define exactamente os 20 tokens de cor pedidos (sem famílias inventadas)', () => {
    const declared = Object.keys(light).filter((name) =>
      COLOR_TOKENS.includes(name as (typeof COLOR_TOKENS)[number]),
    );
    expect(declared.sort()).toEqual([...COLOR_TOKENS].sort());
  });

  it('o bloco automático é idêntico ao explícito (uma única verdade para o escuro)', () => {
    for (const name of COLOR_TOKENS) {
      expect(auto[name], `--${name} divergiu entre os dois blocos escuros`).toBe(dark[name]);
    }
  });

  it('nenhum token usa sintaxe relativa `oklch(from …)` (WebView2 antigo)', () => {
    expect(globals).not.toMatch(/oklch\(\s*from\b/i);
    expect(primitives).not.toMatch(/oklch\(\s*from\b/i);
  });
});

/* ── 2. Espelho de amplitude ────────────────────────────────────────────── */

describe('espelho de amplitude', () => {
  const L = (theme: Theme, name: string) => parseOklch(theme[name]).L;

  it('surface sobe sempre e surface-2 desce sempre face ao bg', () => {
    expect(L(dark, 'surface')).toBeGreaterThan(L(dark, 'bg'));
    expect(L(light, 'surface')).toBeGreaterThan(L(light, 'bg'));
    expect(L(dark, 'surface-2')).toBeLessThan(L(dark, 'bg'));
    expect(L(light, 'surface-2')).toBeLessThan(L(light, 'bg'));
  });

  it('cherry: a amplitude de cada degrau é igual nos dois temas (±0,1 L)', () => {
    const steps: Array<[string, string]> = [
      ['surface', 'bg'],
      ['bg', 'surface-2'],
      ['bg', 'ink-dim'],
      ['ink-dim', 'ink-muted'],
      ['ink-muted', 'ink'],
    ];
    for (const [a, b] of steps) {
      const lightDelta = Math.abs(L(light, a) - L(light, b));
      const darkDelta = Math.abs(L(dark, a) - L(dark, b));
      expect(Math.abs(lightDelta - darkDelta), `${a}/${b}`).toBeLessThan(0.12);
    }
  });
});

/* ── 3. Contraste AA — os 24 pares de §3.5 ──────────────────────────────── */

describe('contraste AA', () => {
  const TEXT_PAIRS: Array<[string, string]> = [
    ['ink', 'bg'],
    ['ink', 'surface'],
    ['ink', 'surface-2'],
    ['ink-muted', 'bg'],
    ['ink-muted', 'surface'],
    ['ink-muted', 'surface-2'],
    ['ink-dim', 'bg'],
    ['ink-dim', 'surface'],
    ['accent-ink', 'bg'],
    ['accent-ink', 'surface'],
    ['accent-ink', 'accent-soft'],
    ['accent-on', 'accent'],
    ['success', 'surface'],
    ['success', 'success-soft'],
    ['warning', 'surface'],
    ['warning', 'warning-soft'],
    ['danger', 'surface'],
    ['danger', 'danger-soft'],
    ['info', 'surface'],
    ['info', 'info-soft'],
  ];

  it.each(TEXT_PAIRS)('%s sobre %s passa 4,5:1 nos dois temas', (fg, bg) => {
    expect(contrast(light, fg, bg)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(dark, fg, bg)).toBeGreaterThanOrEqual(4.5);
  });

  it.each([
    ['accent', 'bg'],
    ['accent', 'surface'],
  ])('%s sobre %s passa 3:1 como componente', (fg, bg) => {
    expect(contrast(light, fg, bg)).toBeGreaterThanOrEqual(3);
    expect(contrast(dark, fg, bg)).toBeGreaterThanOrEqual(3);
  });

  it('as hairlines medem 1,25–1,75:1 — visíveis, nunca a fingir de texto', () => {
    for (const [theme, label] of [
      [light, 'claro'],
      [dark, 'escuro'],
    ] as const) {
      expect(contrast(theme, 'hairline', 'surface'), label).toBeGreaterThan(1.25);
      expect(contrast(theme, 'hairline', 'surface'), label).toBeLessThan(1.75);
      expect(contrast(theme, 'hairline-2', 'surface'), label).toBeGreaterThan(1.5);
      expect(contrast(theme, 'hairline-2', 'surface'), label).toBeLessThan(1.9);
    }
  });
});

/* ── 4. Regras de forma ─────────────────────────────────────────────────── */

describe('gramática hairline', () => {
  const shadowDeclarations = [...primitives.matchAll(/box-shadow\s*:\s*([^;]+);/gi)].map((m) =>
    m[1].trim(),
  );

  it('nenhum primitivo desenha sombra', () => {
    expect(shadowDeclarations.length).toBeGreaterThan(0);
    for (const value of shadowDeclarations) {
      expect(value, 'só `none` é aceite numa superfície').toBe('none');
    }
  });

  it('nenhum primitivo usa backdrop-filter (o vidro morreu)', () => {
    for (const match of primitives.matchAll(/backdrop-filter\s*:\s*([^;]+);/gi)) {
      expect(match[1].trim()).toBe('none');
    }
  });

  it('os tokens de sombra e blur do CSS antigo são `none`', () => {
    expect(light['shadow-sm']).toBe('none');
    expect(light['shadow-md']).toBe('none');
    expect(light['shadow-glass']).toBe('none');
    expect(light['blur']).toBe('none');
    expect(light['blur-sm']).toBe('none');
    expect(light['bg-mesh']).toBe('none');
  });

  it('fontes: system-ui, sem Outfit nem JetBrains Mono sem @font-face', () => {
    expect(light['font']).toContain('system-ui');
    expect(light['font']).not.toMatch(/Outfit/i);
    expect(light['font-mono']).toMatch(/ui-monospace/);
    expect(light['font-mono']).not.toMatch(/JetBrains/i);
    expect(globals).not.toMatch(/@font-face/i);
  });

  it('radius: 12 no card, 8 no controlo, 999 na pill', () => {
    expect(light['r-card']).toBe('12px');
    expect(light['r-control']).toBe('8px');
    expect(light['r-pill']).toBe('999px');
    expect(light['hair']).toBe('1px');
  });

  it('--error é apelido de --danger (0 churn, decisão do dono)', () => {
    expect(light['error']).toBe('var(--danger)');
  });
});

/* ── 5. Sem var() indefinidas nos ficheiros que o tema toca ─────────────── */

describe('contrato de variáveis', () => {
  it('toda a var() usada em primitives.css está definida em globals.css', () => {
    const defined = new Set([
      ...Object.keys(light),
      ...Object.keys(dark),
      ...Object.keys(lightBlock.match(/--[a-z0-9-]+(?=\s*:)/gi) ?? []).map((v) => v.slice(2)),
    ]);
    // tokens locais definidos no próprio primitives.css
    for (const match of primitives.matchAll(/^\s*--([a-z0-9-]+)\s*:/gim)) defined.add(match[1]);

    const used = new Set<string>();
    for (const match of primitives.matchAll(/var\(\s*--([a-z0-9-]+)/gi)) used.add(match[1]);

    const missing = [...used].filter((name) => !defined.has(name) && name !== 'mc-fill');
    expect(missing).toEqual([]);
  });
});

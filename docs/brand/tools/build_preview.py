#!/usr/bin/env python3
"""
Gera docs/brand/preview.html — folha de aprovação visual do logo MiControl.

Faz inline dos SVG (para `currentColor` herdar do CSS) e usa os tokens reais
do tema hairline (`src/styles/globals.css`), para o lado escuro e a sidebar
simulada serem fiéis ao que o dono vê na app.

Uso: python docs/brand/tools/build_preview.py
"""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]  # docs/brand


def inline(name: str, width: int | None = None) -> str:
    """SVG inline com largura fixa (o CSS manda) — o fill currentColor herda."""
    s = (ROOT / name).read_text(encoding="utf-8").strip()
    style = f' style="width:{width}px;height:auto"' if width else ""
    return s.replace("<svg ", f'<svg class="art"{style} ', 1)


TOKENS = """    /* tokens reais do tema hairline — src/styles/globals.css */
    --bg: oklch(96.8% 0.003 250);
    --surface: oklch(99.4% 0.002 250);
    --surface-2: oklch(94.2% 0.003 250);
    --hairline: oklch(90% 0.004 250);
    --hairline-2: oklch(84% 0.004 250);
    --ink: oklch(22% 0.006 250);
    --ink-muted: oklch(38% 0.005 250);
    --ink-dim: oklch(50% 0.005 250);
    --accent: oklch(62% 0.13 268);
    --accent-ink: oklch(46% 0.15 268);
    --accent-soft: oklch(92% 0.035 268);
    --font: system-ui, 'Segoe UI Variable Text', 'Segoe UI', -apple-system, sans-serif;
    --font-mono: ui-monospace, 'Cascadia Code', 'Consolas', monospace;"""

TOKENS_DARK = """    --bg: oklch(18% 0.006 250);
    --surface: oklch(20.6% 0.006 250);
    --surface-2: oklch(15.4% 0.006 250);
    --hairline: oklch(30.8% 0.004 250);
    --hairline-2: oklch(36% 0.004 250);
    --ink: oklch(92.8% 0.005 250);
    --ink-muted: oklch(76.8% 0.006 250);
    --ink-dim: oklch(64.8% 0.005 250);
    --accent: oklch(72% 0.11 268);
    --accent-ink: oklch(80% 0.115 268);
    --accent-soft: oklch(30% 0.05 268);"""

CSS = """  * { box-sizing: border-box; margin: 0; padding: 0; }
  body {
    background: var(--bg);
    color: var(--ink);
    font-family: var(--font);
    font-size: 13px;
    line-height: 1.55;
    padding: 40px 32px 72px;
    max-width: 1080px;
    margin: 0 auto;
  }
  h1 { font-size: 26px; font-weight: 600; letter-spacing: -0.02em; }
  h2 {
    font-size: 11px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.12em;
    color: var(--ink-dim); margin: 44px 0 14px; padding-bottom: 8px;
    border-bottom: 1px solid var(--hairline);
  }
  p { color: var(--ink-muted); max-width: 64ch; }
  code { font-family: var(--font-mono); font-size: 11.5px; color: var(--ink); }
  .lede { margin-top: 8px; }
  .row { display: flex; flex-wrap: wrap; gap: 12px; align-items: stretch; }
  .tile {
    border: 1px solid var(--hairline); border-radius: 12px; background: var(--surface);
    padding: 20px; display: flex; flex-direction: column; gap: 14px;
    flex: 1 1 0; min-width: 300px;
  }
  .tile .cap {
    font-family: var(--font-mono); font-size: 10.5px; color: var(--ink-dim);
    letter-spacing: 0.02em; margin-top: auto;
  }
  .dark .cap { color: oklch(76.8% 0.006 250); }
  .tile .stage { flex: 1; display: flex; align-items: center; justify-content: center; gap: 26px; flex-wrap: wrap; min-height: 96px; }
  .light { background: oklch(99.4% 0.002 250); }
  .dark { background: oklch(18% 0.006 250); color: oklch(92.8% 0.005 250); border-color: oklch(30.8% 0.004 250); }
  .accent-bg { background: oklch(92% 0.035 268); }
  .art { display: block; }
  .ladder { display: flex; align-items: flex-end; gap: 24px; flex-wrap: wrap; }
  .ladder figure { display: flex; flex-direction: column; align-items: center; gap: 8px; }
  .ladder figcaption { font-family: var(--font-mono); font-size: 10px; color: var(--ink-dim); }

  /* sidebar simulada — valores de globals.css */
  .sidebar {
    width: 232px; background: var(--surface); border: 1px solid var(--hairline);
    border-radius: 12px; padding: 10px 10px 16px; display: flex; flex-direction: column; gap: 2px;
  }
  .sidebar-logo {
    display: flex; align-items: center; gap: 10px; padding: 6px 10px 12px;
    font-size: 1rem; font-weight: 700; letter-spacing: -0.4px; color: var(--ink);
  }
  .sidebar-logo svg { width: 16px; height: 16px; flex: 0 0 auto; }
  .nav-item {
    display: flex; align-items: center; gap: 9px; padding: 6px 10px; border-radius: 8px;
    color: var(--ink-muted); font-size: 13px; font-weight: 500; min-height: 26px;
  }
  .nav-item .ic { width: 16px; height: 16px; stroke: currentColor; stroke-width: 1.5; fill: none; flex: 0 0 auto; }
  .nav-item.on { background: var(--accent-soft); color: var(--ink); }
  .nav-group { font-size: 10px; text-transform: uppercase; letter-spacing: 0.1em; color: var(--ink-dim); padding: 12px 10px 4px; }
  .swatches { display: flex; gap: 10px; flex-wrap: wrap; }
  .sw { border: 1px solid var(--hairline); border-radius: 10px; overflow: hidden; width: 176px; background: var(--surface); }
  .sw .chip { height: 64px; }
  .sw .meta { padding: 8px 10px; font-family: var(--font-mono); font-size: 10.5px; color: var(--ink-muted); }
  .muted { color: var(--ink-dim); }
  ul { margin: 0 0 0 18px; color: var(--ink-muted); }
  li { margin: 4px 0; }
"""

NAV = [
    ("bolt", "Desempenho"),
    ("battery", "Bateria"),
    ("monitor", "Ecrã"),
    ("volume", "Áudio"),
    ("broom", "Limpeza"),
    ("shield", "Segurança"),
    ("rocket", "Arranque"),
]
PATHS = {
    "bolt": "M13 3 5 13h6l-1 8 8-10h-6z",
    "battery": "M3 8h13v8H3zM19 10v4",
    "monitor": "M3 5h18v11H3zM9 20h6",
    "volume": "M4 9v6h4l5 4V5L8 9zM17 9a4 4 0 0 1 0 6",
    "broom": "M5 14l5-5 4 4-5 5zM14 5l5 5",
    "shield": "M12 3l7 3v6c0 4-3 7-7 9-4-2-7-5-7-9V6z",
    "rocket": "M12 3c4 2 6 6 6 11l-3 3H9l-3-3c0-5 2-9 6-11zM12 12v.01",
}


def nav_icon(path: str) -> str:
    return (
        '<svg class="ic" viewBox="0 0 24 24" aria-hidden="true">'
        f'<path d="{path}" stroke-linecap="round" stroke-linejoin="round"/></svg>'
    )


def build() -> str:
    b: list[str] = []

    b.append(
        """
  <h1>MiControl — identidade</h1>
  <p class="lede">Folha de aprovação visual. Fundo, tipografia e cor são os tokens reais do tema
  <code>hairline</code>; o painel escuro usa o mesmo <code>[data-theme='dark']</code> da app.</p>
  <p class="muted" style="font-family:var(--font-mono);font-size:10.5px;margin-top:10px">
    docs/brand/preview.html · gerado por docs/brand/tools/build_preview.py</p>
"""
    )

    b.append("<h2>1 · Lockup principal</h2>")
    b.append(
        '<div class="row">'
        f'<div class="tile light"><div class="stage">{inline("logo-primary.svg", 360)}</div>'
        '<div class="cap">claro · placa accent + wordmark em currentColor</div></div>'
        f'<div class="tile dark"><div class="stage">{inline("logo-primary.svg", 360)}</div>'
        '<div class="cap">escuro · o wordmark herda a tinta do tema</div></div>'
        "</div>"
    )
    b.append(
        '<div class="row" style="margin-top:12px">'
        f'<div class="tile accent-bg"><div class="stage">{inline("logo-primary-mono.svg", 340)}</div>'
        '<div class="cap">uma cor · placa em tinta com o M vazado (funciona em qualquer fundo)</div></div>'
        f'<div class="tile"><div class="stage">{inline("logo-wordmark.svg", 300)}</div>'
        '<div class="cap">wordmark sozinho · tudo em currentColor</div></div>'
        f'<div class="tile"><div class="stage">{inline("logo-stacked.svg", 150)}</div>'
        '<div class="cap">lockup vertical · quando a largura não dá</div></div>'
        "</div>"
    )

    b.append("<h2>2 · Símbolo e ícone da app</h2>")
    b.append(
        '<div class="row">'
        f'<div class="tile light"><div class="stage">{inline("logo-mark.svg", 190)}{inline("logo-tile.svg", 92)}</div>'
        '<div class="cap">símbolo (tinta) · ícone (placa accent)</div></div>'
        '<div class="tile light"><div class="ladder">'
        '<figure><img src="icons/128x128@2x.png" width="256" height="256" alt="ícone 256 px"><figcaption>256</figcaption></figure>'
        '<figure><img src="icons/icon.png" width="128" height="128" alt="ícone 128 px"><figcaption>128</figcaption></figure>'
        '<figure><img src="icons/64x64.png" width="64" height="64" alt="ícone 64 px"><figcaption>64</figcaption></figure>'
        '<figure><img src="icons/tray-48.png" width="48" height="48" alt="ícone 48 px"><figcaption>48</figcaption></figure>'
        '<figure><img src="icons/32x32.png" width="32" height="32" alt="ícone 32 px"><figcaption>32</figcaption></figure>'
        '<figure><img src="icons/tray-16.png" width="16" height="16" alt="ícone 16 px"><figcaption>16</figcaption></figure>'
        '</div><div class="cap">cada tamanho rasterizado de raiz a partir do SVG</div></div>'
        "</div>"
    )
    b.append(
        '<div class="row" style="margin-top:12px">'
        '<div class="tile dark"><div class="stage">'
        '<img src="icons/icon.png" width="64" height="64" alt="">'
        '<img src="icons/32x32.png" width="32" height="32" alt="">'
        '<img src="icons/tray-16.png" width="16" height="16" alt="">'
        '</div><div class="cap">barra escura · 64 / 32 / 16</div></div>'
        '<div class="tile light"><div class="stage">'
        '<img src="icons/icon.png" width="64" height="64" alt="">'
        '<img src="icons/32x32.png" width="32" height="32" alt="">'
        '<img src="icons/tray-16.png" width="16" height="16" alt="">'
        '</div><div class="cap">barra clara · 64 / 32 / 16</div></div>'
        "</div>"
    )

    b.append("<h2>3 · Na sidebar da app</h2>")
    nav = "".join(
        f'<div class="nav-item{" on" if name == "Bateria" else ""}">{nav_icon(PATHS[key])}<span>{name}</span></div>'
        for key, name in NAV
    )
    sidebar = (
        '<div class="sidebar">'
        f'<div class="sidebar-logo">{inline("logo-mark.svg")}<span>MiControl</span></div>'
        '<div class="nav-group">Sistema</div>'
        + nav
        + "</div>"
    )
    b.append(f'<div class="row">{sidebar}</div>')
    b.append(
        '<p class="muted" style="margin-top:10px">Aqui o símbolo está a <strong>16 px</strong> para alinhar com '
        "o texto a 1 rem/700 (altura das maiúsculas ≈ 11,5 px). A app usa hoje <code>size={22}</code>: ao lado de "
        "«MiControl» fica 1,6× a altura do texto e pesa demais. Recomendação em BRAND.md §6: mudar para 16.</p>"
    )

    b.append("<h2>4 · Direcções exploradas</h2>")
    b.append(
        '<div class="row">'
        f'<div class="tile"><div class="stage">{inline("concepts/concept-a.svg", 130)}</div>'
        '<div class="cap">A · Detente<br>rejeitada — lê-se «T»/malho</div></div>'
        f'<div class="tile"><div class="stage">{inline("concepts/concept-b.svg", 130)}</div>'
        '<div class="cap">B · M monoline<br>base da direcção escolhida</div></div>'
        f'<div class="tile"><div class="stage">{inline("concepts/concept-c.svg", 130)}</div>'
        '<div class="cap">C · Colarinho<br>rejeitada — lê-se «Pac-Man»/parafuso</div></div>'
        "</div>"
    )

    b.append("<h2>5 · Cor</h2>")
    b.append(
        '<div class="swatches">'
        '<div class="sw"><div class="chip" style="background:#87A2EA"></div>'
        '<div class="meta">accent · #87A2EA<br>oklch(72% .11 268)</div></div>'
        '<div class="sw"><div class="chip" style="background:#191B1D"></div>'
        '<div class="meta">tinta · #191B1D<br>oklch(22% .006 250)</div></div>'
        '<div class="sw"><div class="chip" style="background:#F3F5F6"></div>'
        '<div class="meta">plano claro · #F3F5F6<br>oklch(96.8% .003 250)</div></div>'
        '<div class="sw"><div class="chip" style="background:#15181A"></div>'
        '<div class="meta">plano escuro · #15181A<br>oklch(20.6% .006 250)</div></div>'
        "</div>"
    )

    b.append("<h2>6 · Regras</h2>")
    b.append(
        "<ul>"
        "<li>O símbolo é o M. A placa accent é o ícone da app, não um segundo símbolo.</li>"
        "<li>Nunca pôr o M solto ao lado de «MiControl»: lê-se «MMiControl». Na sidebar o M vive na linha do logo, com o texto à direita.</li>"
        "<li>O accent existe num só sítio: a placa do ícone. Wordmark e símbolo são sempre currentColor. Sem gradientes, sombras ou contornos.</li>"
        "<li>Área de respiro: metade da altura das maiúsculas em todos os lados; nada entra nessa margem.</li>"
        "<li>A 24 px ou menos usar <code>logo-mark-small.svg</code> ou o ícone; o mestre só a partir de 32 px.</li>"
        "</ul>"
    )

    return f"""<!doctype html>
<html lang="pt-PT" data-theme="light">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>MiControl — identidade (folha de aprovação)</title>
<style>
  :root {{
{TOKENS}
  }}
  [data-theme='dark'] {{
{TOKENS_DARK}
  }}
{CSS}
</style>
</head>
<body>
{"".join(b)}
</body>
</html>
"""


def main() -> None:
    html = build()
    (ROOT / "preview.html").write_text(html, encoding="utf-8")
    print("preview.html", len(html), "bytes")


if __name__ == "__main__":
    main()

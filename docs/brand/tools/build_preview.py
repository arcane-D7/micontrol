#!/usr/bin/env python3
"""
Gera docs/brand/preview.html — folha de aprovação visual do logo MiControl.

Regras desta folha (v1.1):
  - **100% self-contained.** Zero referências a ficheiros: os SVG entram como
    markup inline (<svg> colado no HTML) e os PNG rasterizados entram como
    data: URI base64. Um preview que dependa de caminhos relativos parte-se
    quando é mostrado noutro contexto (widget embutido, iframe, cópia em
    %TEMP%) — foi exactamente o que aconteceu na v1.0.
  - Paleta 100% cinza (sem accent): os tokens da UI são os do tema hairline
    com o accent neutralizado, porque a identidade não tem cor de marca.

Uso: python docs/brand/tools/build_preview.py
"""

from __future__ import annotations

import base64
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]  # docs/brand

VERSION = "1.1"


def inline(name: str, width: int | None = None, height: int | None = None) -> str:
    """SVG inline (o fill currentColor herda do CSS); largura/altura via style."""
    s = (ROOT / name).read_text(encoding="utf-8").strip()
    if width:
        h = f"{height}px" if height else "auto"
        style = f' style="width:{width}px;height:{h}"'
    else:
        style = ""
    return s.replace("<svg ", f'<svg class="art"{style} ', 1)


def png(name: str, width: int, height: int | None = None, alt: str = "") -> str:
    """PNG rasterizado embutido como data: URI (nada de ficheiros externos)."""
    data = base64.b64encode((ROOT / name).read_bytes()).decode("ascii")
    h = height if height is not None else width
    return (
        f'<img src="data:image/png;base64,{data}" width="{width}" height="{h}"'
        f' alt="{alt}" style="display:block">'
    )


TOKENS = """    /* tokens do tema hairline, com o accent neutralizado (identidade grayscale) */
    --bg: #f3f5f6;
    --surface: #ffffff;
    --surface-2: #efefef;
    --hairline: #e2e4e6;
    --hairline-2: #d2d5d8;
    --ink: #191b1d;
    --ink-muted: #4a4d50;
    --ink-dim: #6e7174;
    --sel-soft: #e4e6e8;
    --plate: #e8e9ea;
    --plate-dark: #232527;
    --glyph-dark: #f2f3f4;
    --font: system-ui, 'Segoe UI Variable Text', 'Segoe UI', -apple-system, sans-serif;
    --font-mono: ui-monospace, 'Cascadia Code', 'Consolas', monospace;"""

TOKENS_DARK = """    --bg: #15181a;
    --surface: #1b1e20;
    --surface-2: #111314;
    --hairline: #2e3134;
    --hairline-2: #3a3d40;
    --ink: #f2f3f4;
    --ink-muted: #b4b7ba;
    --ink-dim: #86898c;
    --sel-soft: #2c2f32;"""

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
  p { color: var(--ink-muted); max-width: 68ch; }
  code { font-family: var(--font-mono); font-size: 11.5px; color: var(--ink); }
  .lede { margin-top: 8px; }
  .meta-line {
    font-family: var(--font-mono); font-size: 10.5px; color: var(--ink-dim);
    margin-top: 10px;
  }
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
  .dark .cap { color: #b4b7ba; }
  .tile .stage {
    flex: 1; display: flex; align-items: center; justify-content: center;
    gap: 26px; flex-wrap: wrap; min-height: 96px;
  }
  .light { background: #ffffff; }
  .dark { background: #15181a; color: #f2f3f4; border-color: #2e3134; }
  .ink-bg { background: #191b1d; color: #f2f3f4; border-color: #191b1d; }
  .art { display: block; }
  .ladder { display: flex; align-items: flex-end; gap: 24px; flex-wrap: wrap; }
  .ladder figure { display: flex; flex-direction: column; align-items: center; gap: 8px; }
  .ladder figcaption { font-family: var(--font-mono); font-size: 10px; color: var(--ink-dim); }

  /* sidebar simulada — valores de globals.css, accent neutralizado */
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
  .nav-item.on { background: var(--sel-soft); color: var(--ink); }
  .nav-group { font-size: 10px; text-transform: uppercase; letter-spacing: 0.1em; color: var(--ink-dim); padding: 12px 10px 4px; }
  .swatches { display: flex; gap: 10px; flex-wrap: wrap; }
  .sw { border: 1px solid var(--hairline); border-radius: 10px; overflow: hidden; width: 186px; background: var(--surface); }
  .sw .chip { height: 64px; border-bottom: 1px solid var(--hairline); }
  .sw .meta { padding: 8px 10px; font-family: var(--font-mono); font-size: 10.5px; color: var(--ink-muted); }
  .sw .meta b { color: var(--ink); font-weight: 600; }
  table { border-collapse: collapse; width: 100%; max-width: 760px; font-size: 12px; }
  th, td { text-align: left; padding: 6px 10px; border-bottom: 1px solid var(--hairline); }
  th { font-size: 10px; text-transform: uppercase; letter-spacing: 0.1em; color: var(--ink-dim); font-weight: 600; }
  td.num { font-family: var(--font-mono); }
  ul { margin: 0 0 0 18px; color: var(--ink-muted); }
  li { margin: 4px 0; }
  .muted { color: var(--ink-dim); }
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
    icons = ROOT / "icons"
    ico_bytes = f"{(icons / 'icon.ico').stat().st_size:,}".replace(",", "\u202f")

    b.append(
        f"""
  <h1>MiControl — identidade</h1>
  <p class="lede">Folha de aprovação visual. Paleta 100% cinza, sem accent. Fundo, tipografia e
  regras são os tokens do tema <code>hairline</code>; o painel escuro usa o mesmo
  <code>[data-theme='dark']</code> da app.</p>
  <p class="meta-line">docs/brand/preview.html · v{VERSION} · self-contained (SVG inline + PNG em
  data: URI) · gerado por docs/brand/tools/build_preview.py</p>
"""
    )

    b.append("<h2>1 · Lockup principal</h2>")
    b.append(
        '<div class="row">'
        f'<div class="tile light"><div class="stage">{inline("logo-primary.svg", 360)}</div>'
        '<div class="cap">claro · placa #E8E9EA + wordmark em currentColor</div></div>'
        f'<div class="tile dark"><div class="stage">{inline("logo-primary-dark.svg", 360)}</div>'
        '<div class="cap">escuro · placa #232527 + M #F2F3F4</div></div>'
        "</div>"
    )
    b.append(
        '<div class="row" style="margin-top:12px">'
        f'<div class="tile ink-bg"><div class="stage">{inline("logo-primary-mono.svg", 340)}</div>'
        '<div class="cap" style="color:#b4b7ba">uma tinta · placa vazada (evenodd), funciona em qualquer fundo</div></div>'
        f'<div class="tile"><div class="stage">{inline("logo-wordmark.svg", 300)}</div>'
        '<div class="cap">wordmark sozinho · tudo em currentColor</div></div>'
        f'<div class="tile"><div class="stage">{inline("logo-stacked.svg", 140)}{inline("logo-stacked-dark.svg", 140)}</div>'
        '<div class="cap">lockup vertical · versão clara e versão escura</div></div>'
        "</div>"
    )

    b.append("<h2>2 · Símbolo e ícone da app</h2>")
    b.append(
        '<div class="row">'
        f'<div class="tile light"><div class="stage">{inline("logo-mark.svg", 150)}{inline("logo-tile.svg", 92)}{inline("logo-tile-dark.svg", 92)}</div>'
        '<div class="cap">símbolo (currentColor) · tile claro · tile escuro</div></div>'
        '<div class="tile light"><div class="ladder">'
        f'<figure>{inline("logo-tile.svg", 64)}<figcaption>64</figcaption></figure>'
        f'<figure>{inline("logo-tile.svg", 48)}<figcaption>48</figcaption></figure>'
        f'<figure>{inline("logo-tile.svg", 32)}<figcaption>32</figcaption></figure>'
        f'<figure>{inline("logo-tile.svg", 24)}<figcaption>24</figcaption></figure>'
        f'<figure>{inline("logo-tile.svg", 16)}<figcaption>16</figcaption></figure>'
        '</div><div class="cap">escada vectorial inline (o SVG em bruto, não o raster)</div></div>'
        "</div>"
    )

    b.append("<h2>3 · Ícones rasterizados (data: URI)</h2>")
    b.append(
        '<div class="row">'
        '<div class="tile light"><div class="ladder">'
        f'<figure>{png("icons/128x128@2x.png", 256, alt="ícone 256 px")}<figcaption>256</figcaption></figure>'
        f'<figure>{png("icons/128x128.png", 128, alt="ícone 128 px")}<figcaption>128</figcaption></figure>'
        f'<figure>{png("icons/64x64.png", 64, alt="ícone 64 px")}<figcaption>64</figcaption></figure>'
        f'<figure>{png("icons/tray-48.png", 48, alt="ícone 48 px")}<figcaption>48</figcaption></figure>'
        f'<figure>{png("icons/32x32.png", 32, alt="ícone 32 px")}<figcaption>32</figcaption></figure>'
        f'<figure>{png("icons/tray-16.png", 16, alt="ícone 16 px")}<figcaption>16</figcaption></figure>'
        '</div><div class="cap">cada tamanho rasterizado de raiz a partir do SVG (Chrome headless)</div></div>'
        "</div>"
    )
    b.append(
        '<div class="row" style="margin-top:12px">'
        '<div class="tile dark"><div class="stage">'
        f'{png("icons/64x64.png", 64)}'
        f'{png("icons/32x32.png", 32)}'
        f'{png("icons/tray-16.png", 16)}'
        '</div><div class="cap">barra escura · 64 / 32 / 16 (a placa mantém o campo claro)</div></div>'
        '<div class="tile light"><div class="stage">'
        f'{png("icons/64x64.png", 64)}'
        f'{png("icons/32x32.png", 32)}'
        f'{png("icons/tray-16.png", 16)}'
        '</div><div class="cap">barra clara · 64 / 32 / 16 (o M escuro carrega a forma)</div></div>'
        "</div>"
    )
    b.append(
        '<p class="muted" style="margin-top:10px">'
        f"<code>icon.ico</code> — 7 entradas PNG (16, 24, 32, 48, 64, 128, 256), {ico_bytes} bytes; "
        "o ICO não é embutido porque o browser não o escala de forma fiável. As imagens acima são "
        "os PNG reais que o Tauri empacota.</p>"
    )

    b.append("<h2>4 · Na sidebar da app</h2>")
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
    sidebar_dark = (
        '<div class="sidebar" style="background:#1b1e20;border-color:#2e3134;color:#f2f3f4">'
        f'<div class="sidebar-logo" style="color:#f2f3f4">{inline("logo-mark.svg")}<span>MiControl</span></div>'
        '<div class="nav-group" style="color:#86898c">Sistema</div>'
        '<div class="nav-item on" style="color:#f2f3f4;background:#2c2f32">'
        + nav_icon(PATHS["battery"])
        + "<span>Bateria</span></div>"
        + "".join(
            f'<div class="nav-item" style="color:#b4b7ba">{nav_icon(PATHS[k])}<span>{n}</span></div>'
            for k, n in NAV[1:]
        )
        + "</div>"
    )
    b.append(f'<div class="row">{sidebar}{sidebar_dark}</div>')
    b.append(
        '<p class="muted" style="margin-top:10px">Aqui o símbolo está a <strong>16 px</strong> para alinhar com '
        "o texto a 1 rem/700 (altura das maiúsculas ≈ 11,5 px). A app usa hoje <code>size={22}</code>: ao lado de "
        "«MiControl» fica 1,6× a altura do texto e pesa demais. Recomendação em BRAND.md §9: mudar para 16.</p>"
    )

    b.append("<h2>5 · Direcções exploradas</h2>")
    b.append(
        '<div class="row">'
        f'<div class="tile"><div class="stage">{inline("concepts/concept-a.svg", 130)}</div>'
        '<div class="cap">A · Detente<br>rejeitada — lê-se "T"/malho</div></div>'
        f'<div class="tile"><div class="stage">{inline("concepts/concept-b.svg", 130)}</div>'
        '<div class="cap">B · M monoline<br>base da direcção escolhida</div></div>'
        f'<div class="tile"><div class="stage">{inline("concepts/concept-c.svg", 130)}</div>'
        '<div class="cap">C · Colarinho<br>rejeitada — lê-se "Pac-Man"/parafuso</div></div>'
        "</div>"
    )

    b.append("<h2>6 · Cor — paleta cinza</h2>")
    b.append(
        '<div class="swatches">'
        '<div class="sw"><div class="chip" style="background:#191b1d"></div>'
        '<div class="meta"><b>tinta</b> · #191B1D<br>símbolo, wordmark, glifo no tile claro</div></div>'
        '<div class="sw"><div class="chip" style="background:#e8e9ea"></div>'
        '<div class="meta"><b>placa clara</b> · #E8E9EA<br>tile (tema claro + ícone da app)</div></div>'
        '<div class="sw"><div class="chip" style="background:#232527"></div>'
        '<div class="meta"><b>placa escura</b> · #232527<br>tile em superfícies escuras</div></div>'
        '<div class="sw"><div class="chip" style="background:#f2f3f4"></div>'
        '<div class="meta"><b>tinta clara</b> · #F2F3F4<br>M sobre a placa escura</div></div>'
        '<div class="sw"><div class="chip" style="background:#f3f5f6"></div>'
        '<div class="meta"><b>plano claro</b> · #F3F5F6<br>fundo do tema claro</div></div>'
        '<div class="sw"><div class="chip" style="background:#15181a"></div>'
        '<div class="meta"><b>plano escuro</b> · #15181A<br>fundo do tema escuro</div></div>'
        "</div>"
    )
    b.append(
        '<table style="margin-top:16px">'
        "<tr><th>Par</th><th>Rácio</th><th>Leitura</th></tr>"
        '<tr><td>#191B1D sobre #E8E9EA</td><td class="num">14,21:1</td><td>AAA — glifo do tile claro</td></tr>'
        '<tr><td>#F2F3F4 sobre #232527</td><td class="num">13,84:1</td><td>AAA — glifo do tile escuro</td></tr>'
        '<tr><td>#191B1D sobre #FFFFFF</td><td class="num">17,27:1</td><td>AAA — símbolo/wordmark no claro</td></tr>'
        '<tr><td>#E8E9EA sobre #FFFFFF</td><td class="num">1,22:1</td><td>só o bordo da placa — não é contraste de leitura</td></tr>'
        '<tr><td>#232527 sobre #15181A</td><td class="num">1,16:1</td><td>a placa dissolve-se no escuro; o M claro carrega a forma</td></tr>'
        '<tr><td>#232527 sobre #FFFFFF</td><td class="num">15,38:1</td><td>a placa escura salta no claro</td></tr>'
        "</table>"
    )
    b.append(
        '<p class="muted" style="margin-top:10px">Croma máximo dos tokens de marca: 4/255; os cinzas '
        "de UI desta folha ficam ≤ 6/255. Não existe accent de marca: qualquer cor com croma &gt; 6 "
        "é defeito e o <code>svg_audit.py</code> chumba-a.</p>"
    )

    b.append("<h2>7 · Regras</h2>")
    b.append(
        "<ul>"
        "<li>O símbolo é o M. A placa é o ícone da app, não um segundo símbolo.</li>"
        "<li>Nunca pôr o M solto ao lado de «MiControl»: lê-se «MMiControl». Na sidebar o M vive na linha do logo, com o texto à direita.</li>"
        "<li>Sem cor: wordmark e símbolo são sempre currentColor; a placa é cinza neutro. Sem gradientes, sombras ou contornos.</li>"
        "<li>Área de respiro: metade da altura das maiúsculas em todos os lados; nada entra nessa margem.</li>"
        "<li>A 24 px ou menos usar <code>logo-mark-small.svg</code> ou o ícone; o mestre só a partir de 32 px.</li>"
        "<li>Este ficheiro não referencia nada de fora: para o mostrar noutro lado, basta copiá-lo.</li>"
        "</ul>"
    )

    body = "".join(b)
    html = f"""<!doctype html>
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
{body}
</body>
</html>
"""
    return html


def validate(html: str) -> list[str]:
    """Falha o build se houver QUALQUER referência externa. Devolve a lista de problemas."""
    problems: list[str] = []
    for m in re.finditer(r'(?:src|href)\s*=\s*"([^"]*)"', html):
        v = m.group(1)
        if not (v.startswith("data:") or v.startswith("#")):
            problems.append(f"referência externa: {v[:80]}")
    if "<link" in html:
        problems.append("<link> presente")
    if "@import" in html:
        problems.append("@import presente")
    for m in re.finditer(r"url\(([^)]*)\)", html):
        if not m.group(1).strip().strip("'\"").startswith("data:"):
            problems.append(f"url() externo: {m.group(1)[:60]}")
    # xmlns é um namespace, não um fetch
    probe = html.replace('xmlns="http://www.w3.org/2000/svg"', "")
    if re.search(r"https?://", probe):
        problems.append("URL http(s) no documento")
    if "<script" in html:
        problems.append("<script> presente")
    return problems


def main() -> None:
    html = build()
    problems = validate(html)
    out = ROOT / "preview.html"
    out.write_text(html, encoding="utf-8")
    svgs = html.count("<svg ")
    imgs = html.count("<img ")
    print(f"preview.html {len(html):,} bytes · {svgs} SVG inline · {imgs} PNG em data: URI")
    if problems:
        print("FALHA — referências externas:")
        for p in problems:
            print("  -", p)
        raise SystemExit(1)
    print("self-contained: 0 referências externas (src/href/link/@import/url/http)")


if __name__ == "__main__":
    main()
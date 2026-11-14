#!/usr/bin/env python3
"""
Construtor da identidade MiControl — geometria em barras (rect + arco anular).

Uso (da raiz do repo):
    python docs/brand/tools/build_brand.py

Só stdlib. Todas as letras são construídas com primitivas exactas (rect,
arco de círculo, polígono). Nunca há <text>: o wordmark é 100% desenhado,
logo não depende de nenhuma licença de fonte.
"""

from __future__ import annotations

import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]  # docs/brand

# ─────────────────────────────────────────────────────────────────────────────
# Sistema de medida (y para baixo, 0 = topo das maiúsculas, CAP = baseline)
# ─────────────────────────────────────────────────────────────────────────────
CAP = 72.0       # altura das maiúsculas
XH = 50.0        # altura-x
STEM = 13.0      # haste = 18,1% da altura das maiúsculas
CHAMFER = 5.0    # corte a 45° nos cantos exteriores do M
M_W = 78.0       # largura do M (1,08 : 1)
M_VALLEY = 50.0  # vértice interior do V
M_FLAT = 0.0     # face plana no fundo do vale; 0 = bico exacto a 60 graus

INK = "currentColor"
INK_HEX = "#191b1d"
NL = chr(10)  # nova linha, para nao depender de escapes no shell  # = oklch(22% .006 250), o --ink do tema claro
ACCENT = "#87a2ea"  # = oklch(72% .11 268), o --accent do tema escuro — ver BRAND.md §5


def n(v: float) -> str:
    s = f"{v:.3f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def poly(points, close: bool = True) -> str:
    d = "M" + " ".join(f"{n(x)} {n(y)}" for x, y in points[:1])
    d += "".join(f"L{n(x)} {n(y)}" for x, y in points[1:])
    return d + ("Z" if close else "")


def rect(x: float, y: float, w: float, h: float) -> str:
    return poly([(x, y), (x + w, y), (x + w, y + h), (x, y + h)])


def circle_path(cx, cy, r, clockwise=True) -> str:
    sweep = 1 if clockwise else 0
    return (
        f"M{n(cx - r)} {n(cy)}"
        f"A{n(r)} {n(r)} 0 1 {sweep} {n(cx + r)} {n(cy)}"
        f"A{n(r)} {n(r)} 0 1 {sweep} {n(cx - r)} {n(cy)}Z"
    )


def arc_band(cx, cy, r_out, r_in, a0, a1) -> str:
    """Banda anular de a0 a a1 (graus, 0° = direita, y para baixo = sentido horário)."""
    def pt(r, a):
        t = math.radians(a)
        return (cx + r * math.cos(t), cy + r * math.sin(t))

    large = 1 if abs(a1 - a0) > 180 else 0
    p0o, p1o = pt(r_out, a0), pt(r_out, a1)
    p1i, p0i = pt(r_in, a1), pt(r_in, a0)
    return (
        f"M{n(p0o[0])} {n(p0o[1])}"
        f"A{n(r_out)} {n(r_out)} 0 {large} 1 {n(p1o[0])} {n(p1o[1])}"
        f"L{n(p1i[0])} {n(p1i[1])}"
        f"A{n(r_in)} {n(r_in)} 0 {large} 0 {n(p0i[0])} {n(p0i[1])}Z"
    )


# ─────────────────────────────────────────────────────────────────────────────
# M — o monograma
# ─────────────────────────────────────────────────────────────────────────────
def m_points(width=M_W, cap=CAP, stem=STEM, valley=None, chamfer=CHAMFER, flat=M_FLAT):
    """
    M de quatro tracos, com uma regra so.

    - a aresta interior da diagonal esta a 60,00 graus exactos:
      vale = tan(60) * (largura/2 - haste);
    - a espessura horizontal da diagonal e 1,15x a haste, para igualar a
      densidade optica (uma diagonal a 60 parece mais pesada do que e);
    - como consequencia, o bico do vale cai ~1 unidade acima da linha de base
      em qualquer espessura de haste -- nunca sai do corpo da letra;
    - flat > 0 corta o bico por uma face plana horizontal (o detente). A face
      plana desvia as arestas ~2 graus; com flat=0 o angulo fica exacto.
    """
    mid = width / 2
    span = mid - stem
    if valley is None:
        valley = math.tan(math.radians(60.0)) * span
    slope = valley / span
    band = stem * 1.15
    # face inferior paralela a interior: deslocada `band` na horizontal,
    # portanto cruza x=stem em band*slope -- e nao em stem*slope
    under_top = band * slope
    tip_y = under_top + span * slope

    pts = [
        (chamfer, 0),
        (stem, 0),
        (mid, valley),
        (width - stem, 0),
        (width - chamfer, 0),
        (width, chamfer),
        (width, cap - chamfer),
        (width - chamfer, cap),
        (width - stem, cap),
        (width - stem, under_top),
    ]
    if flat > 0:
        hw = flat / 2
        flat_y = tip_y - hw * slope
        pts += [(mid + hw, flat_y), (mid - hw, flat_y)]
    else:
        pts += [(mid, tip_y)]
    pts += [
        (stem, under_top),
        (stem, cap),
        (chamfer, cap),
        (0, cap - chamfer),
        (0, chamfer),
    ]
    return pts


def m_path(**kw) -> str:
    return poly(m_points(**kw))


# ─────────────────────────────────────────────────────────────────────────────
# Letras do wordmark: devolvem (subcaminhos, largura). "dot" = ponto do i.
# ─────────────────────────────────────────────────────────────────────────────
def l_M():
    return [(m_path(), "ink")], M_W


def l_i():
    dot_h, dot_gap = STEM, 9.0
    stem = rect(0, CAP - XH, STEM, XH)
    dot = rect(0, CAP - XH - dot_gap - dot_h, STEM, dot_h)
    return [(stem, "ink"), (dot, "dot")], STEM


def l_C():
    # C de caixa alta: raio 35 em vez de 36 = 1 unidade de overshoot óptico
    r_out = 35.0
    w = 2 * r_out - 4.0
    gap = 44.0
    cy = CAP / 2
    d = arc_band(r_out - 2.0, cy, r_out, r_out - STEM, gap / 2, 360 - gap / 2)
    return [(d, "ink")], w


def l_o():
    w = 52.0
    r_out = w / 2
    cy = CAP - r_out
    d = circle_path(w / 2, cy, r_out) + circle_path(w / 2, cy, r_out - STEM, clockwise=False)
    return [(d, "ink")], w


def l_n():
    w = 51.0
    r_out = w / 2
    cy = CAP - XH + r_out
    d = arc_band(w / 2, cy, r_out, r_out - STEM, 180, 360)
    d += rect(0, cy, STEM, CAP - cy)
    d += rect(w - STEM, cy, STEM, CAP - cy)
    return [(d, "ink")], w


def l_t():
    # a haste sobe 6 acima da barra: sem isso o t lê-se como T
    w = 36.0
    cross_y = CAP - XH - 6.0
    d = rect(w / 2 - STEM / 2, cross_y - 7.0, STEM, CAP - cross_y + 7.0)
    d += rect(0, cross_y, w, STEM)
    return [(d, "ink")], w


def l_r():
    w = 40.0
    r_out, r_in = 26.0, 13.0
    cy = CAP - XH + r_out          # o ombro nasce na linha da altura-x
    d = rect(0, CAP - XH, STEM, XH)  # haste completa: sem isto o braço flutua
    d += arc_band(STEM, cy, r_out, r_in, 270, 360)
    return [(d, "ink")], w


def l_l():
    return [(rect(0, 0, STEM, CAP), "ink")], STEM


LETTERS = {"M": l_M, "i": l_i, "C": l_C, "o": l_o, "n": l_n, "t": l_t, "r": l_r, "l": l_l}

PAIR_SPACE = {
    ("M", "i"): 5.0,
    ("i", "C"): 8.0,
    ("C", "o"): 7.0,
    ("o", "n"): 8.0,
    ("n", "t"): 5.0,
    ("t", "r"): 6.0,
    ("r", "o"): 7.0,
    ("o", "l"): 9.0,
}
SPACE = 9.0


def wordmark(text="MiControl"):
    """[(d, kind, x)] e largura total."""
    out, x, prev = [], 0.0, ""
    for idx, ch in enumerate(text):
        subs, w = LETTERS[ch]()
        for d, kind in subs:
            out.append((d, kind, x))
        x += w
        if idx < len(text) - 1:
            x += PAIR_SPACE.get((prev, ch), SPACE)
        prev = ch
    return out, x


# ─────────────────────────────────────────────────────────────────────────────
# Escrita
# ─────────────────────────────────────────────────────────────────────────────
def svg(vb, content, px_w, title, extra=""):
    """SVG só com viewBox: escala para o contentor (é o que se quer num logo)."""
    x, y, w, h = vb
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{n(x)} {n(y)} {n(w)} {n(h)}" '
        f'role="img" aria-label="{title}">\n'
        f"  <title>{title}</title>{extra}\n{content}</svg>\n"
    )


def write_mark():
    pad = 8.0
    body = f'  <path fill="{INK}" d="{m_path()}"/>\n'
    (ROOT / "logo-mark.svg").write_text(
        svg((-pad, -pad, M_W + 2 * pad, CAP + 2 * pad), body, 256, "MiControl"), encoding="utf-8"
    )


def rounded_rect(x, y, w, h, rad):
    """Rectângulo de cantos redondos, sentido horário."""
    return (
        f"M{n(x + rad)} {n(y)}"
        f"L{n(x + w - rad)} {n(y)}"
        f"A{n(rad)} {n(rad)} 0 0 1 {n(x + w)} {n(y + rad)}"
        f"L{n(x + w)} {n(y + h - rad)}"
        f"A{n(rad)} {n(rad)} 0 0 1 {n(x + w - rad)} {n(y + h)}"
        f"L{n(x + rad)} {n(y + h)}"
        f"A{n(rad)} {n(rad)} 0 0 1 {n(x)} {n(y + h - rad)}"
        f"L{n(x)} {n(y + rad)}"
        f"A{n(rad)} {n(rad)} 0 0 1 {n(x + rad)} {n(y)}Z"
    )


def tile_paths(size=256.0, pad=10.0, rad=56.0):
    """Devolve (plate_d, glyph_d) da placa accent com o M em tinta."""
    plate = rounded_rect(pad, pad, size - 2 * pad, size - 2 * pad, rad)
    bold = dict(stem=16.0, flat=0.0, chamfer=6.0)
    scale = 0.56 * (size - 2 * pad) / M_W
    mw, mh = M_W * scale, CAP * scale
    tx = pad + (size - 2 * pad - mw) / 2
    ty = pad + (size - 2 * pad - mh) / 2 - size * 0.008
    glyph = poly([(tx + px * scale, ty + py * scale) for px, py in m_points(**bold)])
    return plate, glyph


def write_tile(name="logo-tile.svg"):
    """
    Ícone da app: placa accent (squircle 22%) com o M em tinta.

    Decisão (ver BRAND.md §4): o glifo escuro sobre placa clara ganha ao M
    vazado a 16–32 px — o vazado muda de cor com a barra de tarefas e perde
    contraste; a placa redonda é a forma nativa do Windows 11.
    O M do ícone é o corte «small»: haste a 22% da altura (vs 18% do mestre),
    porque a 16 px só a espessura e o contraste sobrevivem.
    """
    plate, glyph = tile_paths()
    body = (
        f'  <path fill="{ACCENT}" d="{plate}"/>\n'
                f'  <path fill="{INK_HEX}" d="{glyph}"/>\n'
    )
    (ROOT / name).write_text(svg((0, 0, 256, 256), body, 256, "MiControl"), encoding="utf-8")


def write_wordmark():
    subs, total = wordmark()
    pad = 8.0
    body = "".join(
        f'  <path fill="{INK}" transform="translate({n(x)} 0)" d="{d}"/>\n' for d, _, x in subs
    )
    (ROOT / "logo-wordmark.svg").write_text(
        svg((-pad, -pad, total + 2 * pad, CAP + 2 * pad), body, int((total + 2 * pad) * 3), "MiControl"),
        encoding="utf-8",
    )


def write_primary(name="logo-primary.svg", mono=False):
    """
    Lockup horizontal: placa (altura = altura das maiusculas) + wordmark.

    O simbolo e a placa, nao o M solto: um M solto ao lado de «MiControl»
    le-se como «MMiControl». A placa e um objecto, nao uma letra.

    mono=True -> uma so tinta (currentColor): a placa leva o M vazado a
    fill-rule="evenodd", portanto funciona sobre qualquer fundo.
    """
    subs, total = wordmark()
    gap, pad = 24.0, 10.0
    x0 = CAP + gap
    plate, glyph = tile_paths(size=CAP, pad=0.0, rad=CAP * 0.22)
    if mono:
        body = f'  <path fill="{INK}" fill-rule="evenodd" d="{plate}{glyph}"/>' + NL
    else:
        body = f'  <path fill="{ACCENT}" d="{plate}"/>' + NL
        body += f'  <path fill="{INK_HEX}" d="{glyph}"/>' + NL
    for d, _kind, x in subs:
        body += f'  <path fill="{INK}" transform="translate({n(x0 + x)} 0)" d="{d}"/>' + NL
    w = x0 + total
    (ROOT / name).write_text(
        svg((-pad, -pad, w + 2 * pad, CAP + 2 * pad), body, int((w + 2 * pad) * 2), "MiControl"),
        encoding="utf-8",
    )


def write_stacked(name="logo-stacked.svg"):
    """Lockup vertical: placa em cima, wordmark centrado em baixo."""
    subs, total = wordmark()
    plate, glyph = tile_paths(size=CAP, pad=0.0, rad=CAP * 0.22)
    gap, pad = 28.0, 10.0
    w = max(CAP, total)
    x_plate = (w - CAP) / 2
    y_word = CAP + gap
    body = f'  <g transform="translate({n(x_plate)} 0)">' + NL
    body += f'    <path fill="{ACCENT}" d="{plate}"/>' + NL
    body += f'    <path fill="{INK_HEX}" d="{glyph}"/>' + NL
    body += "  </g>" + NL
    x0 = (w - total) / 2
    for d, _kind, x in subs:
        body += f'  <path fill="{INK}" transform="translate({n(x0 + x)} {n(y_word)})" d="{d}"/>' + NL
    (ROOT / name).write_text(
        svg((-pad, -pad, w + 2 * pad, y_word + CAP + 2 * pad), body, 400, "MiControl"),
        encoding="utf-8",
    )


def write_concept_b():
    """A direccao B tal como ficou (para a folha de exploracao nao mostrar o esboco)."""
    d = m_path()
    out = ROOT / "concepts" / "concept-b.svg"
    out.parent.mkdir(exist_ok=True)
    out.write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="-8 -8 94 88" role="img" aria-label="Conceito B">'
        + NL
        + f'<path fill="#111" d="{d}"/>'
        + NL
        + "</svg>"
        + NL,
        encoding="utf-8",
    )


def main():
    write_mark()
    write_tile()
    write_wordmark()
    write_primary()
    write_primary(name="logo-primary-mono.svg", mono=True)
    write_stacked()
    write_concept_b()
    print("svg:", sorted(p.name for p in ROOT.glob("logo-*.svg")))


if __name__ == "__main__":
    main()

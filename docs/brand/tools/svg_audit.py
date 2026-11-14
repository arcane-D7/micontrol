#!/usr/bin/env python3
"""
Auditoria heurística da identidade MiControl.

Verifica, para cada SVG final, o que a marca promete e uma ferramenta consegue
provar: geometria vectorial pura (sem raster, texto, filtros ou gradientes),
viewBox presente, acessibilidade mínima, paleta 100% cinza (croma <= 6) dentro
da paleta documentada, e contraste AA do glifo do ícone contra a placa. Audita
também preview.html (self-containment: zero referências externas).

Uso:
    python docs/brand/tools/svg_audit.py            # texto legível
    python docs/brand/tools/svg_audit.py --json     # máquina

Saída: tabela com pontuação 0–100 por ficheiro. Exit 1 se algum ficheiro
ficar abaixo de 90 ou tiver uma falha dura.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]  # docs/brand

PALETTE = {
    "#191b1d": "tinta",
    "#e8e9ea": "placa clara",
    "#232527": "placa escura",
    "#f2f3f4": "tinta clara",
    "#f3f5f6": "plano claro",
    "#15181a": "plano escuro",
    "#111": "preto de exploração (concepts/)",
}
MAX_CHROMA = 6  # 0–255 por canal; acima disto já se lê como cor, não como cinza

HARD = {
    "sem viewBox": 30,
    "<text>": 20,
    "<image> (raster embutido)": 20,
    "<filter>": 15,
    "<mask>": 15,
    "gradiente": 15,
    "<style> externo/inline": 10,
    "<script>": 30,
    "referência externa": 30,
    "cor com croma > 6": 25,
    "sem paths": 40,
}
SOFT = {
    "sem <title>": 5,
    "sem role/aria-label": 3,
    "width/height fixos na raiz": 5,
    "stroke em logo de marca": 5,
    "hex fora da paleta": 2,
    "d de path inválido": 10,
    "contraste do tile < 4,5:1": 20,
    "sem currentColor": 5,
    "ficheiro > 40 KB": 5,
}


def rgb(hex_color: str) -> tuple[int, int, int]:
    h = hex_color.lstrip("#")
    if len(h) == 3:
        h = "".join(c * 2 for c in h)
    return tuple(int(h[i : i + 2], 16) for i in (0, 2, 4))  # type: ignore[return-value]


def chroma(hex_color: str) -> int:
    r, g, b = rgb(hex_color)
    return max(r, g, b) - min(r, g, b)


def luminance(hex_color: str) -> float:
    def f(c: int) -> float:
        s = c / 255
        return s / 12.92 if s <= 0.04045 else ((s + 0.055) / 1.055) ** 2.4

    r, g, b = (f(c) for c in rgb(hex_color))
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(a: str, b: str) -> float:
    la, lb = luminance(a), luminance(b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


def hexes(text: str) -> list[str]:
    """Todas as cores hex do documento (atributos, CSS, estilos inline), por ordem."""
    return [m.group(1).lower() for m in re.finditer(r"(#(?:[0-9a-fA-F]{6}|[0-9a-fA-F]{3}))\b", text)]


def audit_svg(path: Path) -> dict:
    rel = str(path.relative_to(ROOT)).replace("\\", "/")
    text = path.read_text(encoding="utf-8")
    findings: list[str] = []
    score = 100

    def hit(kind: str, detail: str = "") -> None:
        nonlocal score
        table = HARD if kind in HARD else SOFT
        score -= table[kind]
        findings.append(f"{kind}{': ' + detail if detail else ''}")

    # Estrutura
    if not re.search(r'<svg[^>]*viewBox="', text):
        hit("sem viewBox")
    root = re.search(r"<svg[^>]*>", text)
    if root and re.search(r'\swidth="', root.group(0)):
        hit("width/height fixos na raiz")
    if "<title>" not in text:
        hit("sem <title>")
    if 'role="img"' not in text or "aria-label=" not in text:
        hit("sem role/aria-label")

    # Proibições da marca
    for tag, kind in [
        ("<text", "<text>"),
        ("<image", "<image> (raster embutido)"),
        ("<filter", "<filter>"),
        ("<mask", "<mask>"),
        ("<script", "<script>"),
        ("<style", "<style> externo/inline"),
    ]:
        if tag in text:
            hit(kind)
    if re.search(r"<(linear|radial)Gradient", text):
        hit("gradiente")
    if re.search(r'(?:href|src)\s*=\s*"(?!#|data:)', text):
        hit("referência externa")
    if re.search(r'\sstroke="', text) and rel.startswith("logo-"):
        hit("stroke em logo de marca")

    # Paths
    ds = re.findall(r'\sd="([^"]*)"', text)
    if not ds:
        hit("sem paths")
    for d in ds:
        if (
            "NaN" in d
            or "None" in d
            or not re.match(r"^[Mm]", d)
            or re.search(r"[^MmZzLlHhVvCcSsQqTtAa0-9.,\-\s]", d)
        ):
            hit("d de path inválido", d[:40])
            break

    # Cor
    colors = hexes(text)
    for c in colors:
        if chroma(c) > MAX_CHROMA:
            hit("cor com croma > 6", f"{c} (croma {chroma(c)})")
        elif c not in PALETTE:
            hit("hex fora da paleta", c)
    if rel.startswith("logo-"):
        theme_files = ("logo-mark", "logo-wordmark", "logo-primary", "logo-stacked")
        if any(rel.startswith(p) for p in theme_files) and 'fill="currentColor"' not in text:
            hit("sem currentColor")

    # Contraste do tile (primeira cor = placa, segunda = glifo)
    if rel.startswith("logo-tile") and len(colors) >= 2:
        ratio = contrast(colors[0], colors[1])
        if ratio < 4.5:
            hit("contraste do tile < 4,5:1", f"{ratio:.2f}:1")

    size_kb = path.stat().st_size / 1024
    if size_kb > 40:
        hit("ficheiro > 40 KB", f"{size_kb:.1f} KB")

    score = max(0, score)
    return {
        "file": rel,
        "score": score,
        "pass": score >= 90 and not any(f.split(":")[0].strip() in HARD for f in findings),
        "findings": findings,
        "colors": sorted(set(colors)),
        "colors_named": [PALETTE.get(c, c) for c in sorted(set(colors))],
    }


def audit_preview(path: Path) -> dict:
    rel = str(path.relative_to(ROOT)).replace("\\", "/")
    if not path.exists():
        return {"file": rel, "score": 0, "pass": False, "findings": ["não existe"], "metrics": {}}
    html = path.read_text(encoding="utf-8")
    findings: list[str] = []
    score = 100
    external = []
    for m in re.finditer(r'(?:src|href)\s*=\s*"([^"]*)"', html):
        v = m.group(1)
        if not (v.startswith("data:") or v.startswith("#")):
            external.append(v[:60])
    if external:
        score -= 40
        findings.append(f"referências externas: {external[:3]}")
    for token, cost in (("<link", 30), ("@import", 30), ("<script", 30)):
        if token in html:
            score -= cost
            findings.append(f"{token} presente")
    for m in re.finditer(r"url\(([^)]*)\)", html):
        if not m.group(1).strip().strip("'\"").startswith("data:"):
            score -= 30
            findings.append(f"url() externo: {m.group(1)[:40]}")
    probe = html.replace('xmlns="http://www.w3.org/2000/svg"', "")
    if re.search(r"https?://", probe):
        score -= 30
        findings.append("URL http(s) no documento")
    svgs = html.count("<svg ")
    pngs = html.count('src="data:image/png;base64,')
    if svgs == 0:
        score -= 40
        findings.append("sem SVG inline")
    if pngs == 0:
        score -= 20
        findings.append("sem PNG em data: URI")
    # paleta: qualquer hex com croma > 6 é falha (todo o documento, incluindo CSS)
    bad = sorted({c for c in hexes(html) if chroma(c) > MAX_CHROMA})
    if bad:
        score -= 30
        findings.append(f"cor com croma > 6: {bad[:4]}")
    score = max(0, score)
    return {
        "file": rel,
        "score": score,
        "pass": score >= 90,
        "findings": findings,
        "metrics": {"svg_inline": svgs, "png_data_uri": pngs, "bytes": len(html)},
    }


def main() -> None:
    as_json = "--json" in sys.argv
    svgs = sorted(ROOT.glob("logo-*.svg")) + sorted((ROOT / "concepts").glob("*.svg"))
    results = [audit_svg(p) for p in svgs]
    preview = ROOT / "preview.html"
    if preview.exists():
        results.append(audit_preview(preview))

    if as_json:
        print(json.dumps(results, ensure_ascii=False, indent=2))
    else:
        print(f"{'ficheiro':<28} {'nota':>5}  estado")
        print("-" * 72)
        for r in results:
            state = "OK" if r["pass"] else "FALHA"
            print(f"{r['file']:<28} {r['score']:>3}/100  {state}")
            for f in r["findings"]:
                print(f"    · {f}")
            if r.get("metrics"):
                m = r["metrics"]
                print(f"    · {m['svg_inline']} SVG inline · {m['png_data_uri']} PNG data: URI · {m['bytes']:,} bytes")
        failed = [r for r in results if not r["pass"]]
        print("-" * 72)
        print(f"{len(results) - len(failed)}/{len(results)} ficheiros passam · croma máx. permitido {MAX_CHROMA}/255")

    if any(not r["pass"] for r in results):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
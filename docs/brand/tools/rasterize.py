#!/usr/bin/env python3
"""
Rasterização do logo MiControl: SVG -> PNG (Chrome headless) -> ICO multi-tamanho.

Método (documentado também em docs/brand/BRAND.md §7):
  - O SVG mestre tem só viewBox. Para rasterizar, o SVG é embutido num HTML a
    100% da caixa, no tamanho exacto em px, e o Chrome headless tira um
    screenshot com fundo transparente. Nesta máquina não há cairo, rsvg,
    Inkscape nem ImageMagick: o Chrome é o rasterizador disponível e é também
    o mais fiel ao que a app (WebView2) vai desenhar.
  - Todos os tamanhos de uma família são desenhados numa só página e
    recortados por coordenadas — uma única arranque do Chrome por família,
    em vez de um por ícone.
  - O .ico é escrito à mão (ICONDIR + entradas PNG), para cada tamanho ser
    rasterizado de raiz em vez de reduzido a partir de um 256.

Uso (da raiz do repo):
    python docs/brand/tools/rasterize.py
Escreve docs/brand/icons/*  (staging; NÃO toca em src-tauri/icons).
"""

from __future__ import annotations

import base64
import os
import struct
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]          # docs/brand
ICONS = ROOT / "icons"
SCRATCH = Path(tempfile.gettempdir()) / "micontrol-brand-raster"

CHROME_CANDIDATES = [
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
]


def find_chrome() -> str:
    for c in CHROME_CANDIDATES:
        if os.path.exists(c):
            return c
    raise RuntimeError("sem Chrome/Edge para rasterizar")


def _chrome_shot_once(html: str, png: Path, w: int, h: int, attempt: int) -> None:
    chrome = find_chrome()
    SCRATCH.mkdir(parents=True, exist_ok=True)
    html_path = SCRATCH / "render.html"
    html_path.write_text(html, encoding="utf-8")
    shot = SCRATCH / f"shot-{w}-{h}-{attempt}.png"
    shot.unlink(missing_ok=True)
    cmd = [
        chrome, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-first-run",
        "--no-default-browser-check", "--use-mock-keychain", "--password-store=basic",
        "--disable-extensions", "--force-device-scale-factor=1",
        "--default-background-color=00000000",
        f"--window-size={w},{h}", f"--screenshot={shot}",
        f"--user-data-dir={SCRATCH / ('profile-%d' % attempt)}", "file://" + str(html_path.resolve()),
    ]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.time() + 60
    last = -1
    while time.time() < deadline:
        if shot.exists():
            size = shot.stat().st_size
            if size > 0 and size == last:
                break
            last = size
        if proc.poll() is not None and shot.exists():
            break
        time.sleep(0.15)
    try:
        proc.wait(timeout=1)
    except subprocess.TimeoutExpired:
        proc.kill()
    if not shot.exists():
        raise RuntimeError("chrome não produziu imagem")
    os.replace(shot, png)


def _chrome_shot(html: str, png: Path, w: int, h: int) -> None:
    last: Exception | None = None
    for attempt in range(3):
        try:
            _chrome_shot_once(html, png, w, h, attempt)
            return
        except RuntimeError as exc:
            last = exc
            time.sleep(0.6)
    raise last  # type: ignore[misc]


def _wrap(img_html: str, w: int, h: int) -> str:
    return (
        "<!doctype html><html><head><meta charset='utf-8'><style>"
        f"html,body{{margin:0;padding:0;background:transparent;width:{w}px;height:{h}px;overflow:hidden}}"
        "svg{display:block}"
        "</style></head><body>"
        f"{img_html}</body></html>"
    )


def screenshot_file(html_path: Path, png: Path, w: int, h: int) -> None:
    """Screenshot de um HTML já no sítio (para os caminhos relativos resolverem)."""
    last: Exception | None = None
    chrome = find_chrome()
    for attempt in range(3):
        try:
            SCRATCH.mkdir(parents=True, exist_ok=True)
            shot = SCRATCH / f"shot-file-{w}-{attempt}.png"
            shot.unlink(missing_ok=True)
            cmd = [
                chrome, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-first-run",
                "--no-default-browser-check", "--use-mock-keychain", "--password-store=basic",
                "--disable-extensions", "--force-device-scale-factor=1",
                f"--window-size={w},{h}", f"--screenshot={shot}",
                f"--user-data-dir={SCRATCH / ('profile-f%d' % attempt)}",
                "file://" + str(html_path.resolve()),
            ]
            proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            deadline = time.time() + 60
            last_size = -1
            while time.time() < deadline:
                if shot.exists():
                    size = shot.stat().st_size
                    if size > 0 and size == last_size:
                        break
                    last_size = size
                if proc.poll() is not None and shot.exists():
                    break
                time.sleep(0.15)
            try:
                proc.wait(timeout=1)
            except subprocess.TimeoutExpired:
                proc.kill()
            if not shot.exists():
                raise RuntimeError("chrome não produziu imagem")
            os.replace(shot, png)
            return
        except RuntimeError as exc:
            last = exc
            time.sleep(0.6)
    raise last  # type: ignore[misc]


def render_svg(svg_text: str, png: Path, w: int, h: int) -> None:
    """Desenha um SVG num PNG exacto de w×h (o SVG deve ter esse aspect ratio)."""
    sized = svg_text.replace("<svg ", f'<svg width="{w}" height="{h}" ', 1)
    png.parent.mkdir(parents=True, exist_ok=True)
    _chrome_shot(_wrap(sized, w, h), png, w, h)


def render_batch(items: list[tuple[str, str, int, int]], out_dir: Path, gap: int = 10) -> dict[str, Path]:
    """
    Desenha várias imagens numa só página e recorta-as.

    items = [(nome, svg_text, w, h)] — cada uma na sua linha, com `gap` de
    respiro. Devolve {nome: png}.
    """
    out_dir.mkdir(parents=True, exist_ok=True)
    parts, boxes, y = [], [], 0
    page_w = 0
    for name, svg, w, h in items:
        parts.append(
            f'<div style="position:absolute;left:0;top:{y}px;width:{w}px;height:{h}px">'
            + svg.replace("<svg ", f'<svg width="{w}" height="{h}" ', 1)
            + "</div>"
        )
        boxes.append((name, 0, y, w, h))
        page_w = max(page_w, w)
        y += h + gap
    page_h = max(y - gap, 1)
    html = (
        "<!doctype html><html><head><meta charset='utf-8'><style>"
        f"html,body{{margin:0;padding:0;background:transparent;width:{page_w}px;height:{page_h}px;overflow:hidden}}"
        "svg{display:block}div{position:absolute}"
        "</style></head><body>" + "".join(parts) + "</body></html>"
    )
    sheet = SCRATCH / "batch.png"
    _chrome_shot(html, sheet, page_w, page_h)
    from PIL import Image

    img = Image.open(sheet).convert("RGBA")
    out: dict[str, Path] = {}
    for name, x, yy, w, h in boxes:
        crop = img.crop((x, yy, x + w, yy + h))
        p = out_dir / name
        crop.save(p)
        out[name] = p
    return out


def write_ico(pngs: list[Path], out: Path) -> None:
    """ICO com entradas PNG (Vista+)."""
    blobs = [p.read_bytes() for p in pngs]
    count = len(blobs)
    header = struct.pack("<HHH", 0, 1, count)
    offset = 6 + 16 * count
    entries, data = b"", b""
    for png, blob in zip(pngs, blobs):
        with open(png, "rb") as fh:
            head = fh.read(24)
        w, h = struct.unpack(">II", head[16:24])
        entries += struct.pack(
            "<BBBBHHII", 0 if w >= 256 else w, 0 if h >= 256 else h, 0, 0, 1, 32, len(blob), offset
        )
        offset += len(blob)
        data += blob
    out.write_bytes(header + entries + data)


def main() -> None:
    import build_brand as b  # mesma pasta

    ICONS.mkdir(parents=True, exist_ok=True)
    tmp = ICONS / "_tmp"
    tile = (ROOT / "logo-tile.svg").read_text(encoding="utf-8")

    # 1. Ícone da app — placa accent com o M, rasterizada de raiz por tamanho
    app_sizes = [512, 256, 128, 64, 48, 32, 24, 16]
    got = render_batch([(f"tile-{s}.png", tile, s, s) for s in app_sizes], tmp)
    (ICONS / "icon.png").write_bytes(got["tile-512.png"].read_bytes())
    (ICONS / "128x128@2x.png").write_bytes(got["tile-256.png"].read_bytes())
    (ICONS / "128x128.png").write_bytes(got["tile-128.png"].read_bytes())
    (ICONS / "64x64.png").write_bytes(got["tile-64.png"].read_bytes())
    (ICONS / "32x32.png").write_bytes(got["tile-32.png"].read_bytes())
    write_ico([got[f"tile-{s}.png"] for s in (16, 24, 32, 48, 64, 128, 256)], ICONS / "icon.ico")

    # 2. Windows Store / MSIX
    store_sizes = [310, 284, 150, 142, 107, 89, 71, 50, 44, 30]
    got2 = render_batch([(f"s{s}.png", tile, s, s) for s in store_sizes], tmp)
    for s in (30, 44, 71, 89, 107, 142, 150, 284, 310):
        (ICONS / f"Square{s}x{s}Logo.png").write_bytes(got2[f"s{s}.png"].read_bytes())
    (ICONS / "StoreLogo.png").write_bytes(got2["s50.png"].read_bytes())

    # 3. Tray — o mesmo ícone, em 16/24/32/48. A placa accent garante contraste
    #    tanto na barra clara como na escura; um M solto em periwinkle
    #    desaparece em fundo claro (medido no preview) e um M em tinta
    #    desaparece em fundo escuro.
    got3 = render_batch([(f"tray-{s}.png", tile, s, s) for s in (48, 32, 24, 16)], tmp)
    for s in (16, 24, 32, 48):
        (ICONS / f"tray-{s}.png").write_bytes(got3[f"tray-{s}.png"].read_bytes())

    # corte pequeno do símbolo (haste 22%), para usos mono a 16–24 px
    small = b.m_path(width=b.M_W, stem=17.0, flat=0.0, chamfer=0.0)
    (ROOT / "logo-mark-small.svg").write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="-2 -2 {b.M_W + 4} {b.CAP + 4}" role="img" aria-label="MiControl"><title>MiControl</title>'
        f'<path fill="currentColor" d="{small}"/></svg>\n',
        encoding="utf-8",
    )

    for p in tmp.glob("*"):
        p.unlink()
    tmp.rmdir()
    print("ícones em", ICONS)
    print("  ", sorted(p.name for p in ICONS.iterdir()))


if __name__ == "__main__":
    main()
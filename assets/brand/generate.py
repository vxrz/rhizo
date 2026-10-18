#!/usr/bin/env python3
"""Generates the rhizo brand SVGs from one set of shapes and colors.

    python3 assets/brand/generate.py

The app icon (icon.svg) is hand-tuned and not generated; this script builds
everything derived from it: marks, wordmarks, lockups, favicon, social card.
"""

from pathlib import Path

OUT = Path(__file__).parent

INK = "#14131f"
PAPER = "#f8f8fc"

# Branch colors, straight from the product themes (crates/core/src/theme.rs).
DARK = dict(indigo="#818cf8", sky="#38bdf8", emerald="#34d399", amber="#fbbf24", pink="#f472b6")
LIGHT = dict(indigo="#6366f1", sky="#0ea5e9", emerald="#10b981", amber="#f59e0b", pink="#ec4899")

# The mark, in icon coordinates (see icon.svg), without its translate.
SHOOTS = [
    ("indigo", "M380 512 C 500 512, 500 292, 610 292", (646, 292, 46)),
    ("sky", "M380 512 L 660 512", (702, 512, 46)),
    ("emerald", "M380 512 C 490 512, 490 736, 590 736", (626, 736, 46)),
]
SPROUTS = [
    ("amber", "M650 292 C 720 292, 718 206, 780 206", (804, 206, 30)),
    ("pink", "M630 736 C 700 736, 698 812, 760 812", (784, 812, 30)),
]
CROSS_LINK = "M736 548 C 790 610, 800 700, 790 770"
SEED = (356, 512)
MARK_VIEWBOX = "236 152 600 712"


def mark(colors, seed_fill, seed_core, link_color, link_opacity, shoot_from=None):
    """The mark's shapes. `shoot_from` fades each shoot in from that color."""
    defs, body = [], []
    for name, d, (cx, cy, _) in SHOOTS:
        stroke = colors[name]
        if shoot_from:
            gid = f"s-{name}-{seed_fill[1:]}"
            defs.append(
                f'<linearGradient id="{gid}" gradientUnits="userSpaceOnUse" x1="380" y1="512" x2="{cx - 36}" y2="{cy}">'
                f'<stop offset="0" stop-color="{shoot_from}"/><stop offset="1" stop-color="{stroke}"/></linearGradient>'
            )
            stroke = f"url(#{gid})"
        body.append(f'<path d="{d}" fill="none" stroke="{stroke}" stroke-width="38" stroke-linecap="round"/>')
    for name, d, _ in SPROUTS:
        body.append(f'<path d="{d}" fill="none" stroke="{colors[name]}" stroke-width="22" stroke-linecap="round"/>')
    body.append(
        f'<path d="{CROSS_LINK}" fill="none" stroke="{link_color}" stroke-opacity="{link_opacity}" '
        f'stroke-width="12" stroke-linecap="round" stroke-dasharray="2 26"/>'
    )
    for name, _, (cx, cy, r) in SHOOTS + SPROUTS:
        body.append(f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{colors[name]}"/>')
    sx, sy = SEED
    body.append(f'<circle cx="{sx}" cy="{sy}" r="88" fill="{seed_fill}"/>')
    body.append(f'<circle cx="{sx}" cy="{sy}" r="30" fill="{seed_core}"/>')
    return "".join(defs), "\n  ".join(body)


def mono_mark():
    """Single color (currentColor). The seed is a ring so it stays a seed."""
    parts = []
    for _, d, _ in SHOOTS:
        # Start at the seed's rim so the round caps don't fill the ring's hole.
        d = d.replace("M380 512", "M444 512", 1)
        parts.append(f'<path d="{d}" fill="none" stroke="currentColor" stroke-width="38" stroke-linecap="butt"/>')
    for _, d, _ in SPROUTS:
        parts.append(f'<path d="{d}" fill="none" stroke="currentColor" stroke-width="22" stroke-linecap="round"/>')
    parts.append(
        f'<path d="{CROSS_LINK}" fill="none" stroke="currentColor" stroke-opacity="0.5" stroke-width="12" '
        'stroke-linecap="round" stroke-dasharray="2 26"/>'
    )
    for _, _, (cx, cy, r) in SHOOTS + SPROUTS:
        parts.append(f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="currentColor"/>')
    sx, sy = SEED
    # Ring: outer circle minus inner circle, via evenodd.
    parts.append(
        f'<path fill="currentColor" fill-rule="evenodd" d="M{sx - 88} {sy} a88 88 0 1 0 176 0 a88 88 0 1 0 -176 0 '
        f'M{sx - 30} {sy} a30 30 0 1 0 60 0 a30 30 0 1 0 -60 0"/>'
    )
    return "\n  ".join(parts)


# Monoline wordmark: x-height 60, stroke 14, baseline 100, round everything.
# The tittle of the i is a node.
WORD_VIEWBOX = (0, 0, 312, 110)


def wordmark(ink, accent):
    return f"""<g fill="none" stroke="{ink}" stroke-width="14" stroke-linecap="round" stroke-linejoin="round">
    <path d="M7 40 L7 100 M7 64 C7 48 17 40 33 40"/>
    <path d="M63 10 L63 100 M63 64 C63 48 73 40 89 40 C105 40 113 48 113 64 L113 100"/>
    <path d="M143 44 L143 100"/>
    <path d="M173 40 L213 40 L173 100 L213 100"/>
    <circle cx="273" cy="70" r="30"/>
  </g>
  <circle cx="143" cy="17" r="11" fill="{accent}"/>"""


def svg(viewbox, body, title="rhizo", extra_defs="", w=None, h=None):
    size = f' width="{w}" height="{h}"' if w else ""
    defs = f"\n  <defs>{extra_defs}</defs>" if extra_defs else ""
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{viewbox}"{size} role="img" aria-label="{title}">\n'
        f"  <title>{title}</title>{defs}\n  {body}\n</svg>\n"
    )


def nested(x, y, width, height, viewbox, content):
    return f'<svg x="{x}" y="{y}" width="{width}" height="{height}" viewBox="{viewbox}">{content}</svg>'


def write(name, text):
    (OUT / name).write_text(text)
    print("wrote", name)


def main():
    # Marks
    d_defs, d_body = mark(DARK, PAPER, LIGHT["indigo"], "#f5f3ff", 0.45, shoot_from="#e0e7ff")
    l_defs, l_body = mark(LIGHT, INK, LIGHT["indigo"], INK, 0.3)
    write("mark-dark.svg", svg(MARK_VIEWBOX, d_body, "rhizo mark (for dark backgrounds)", d_defs))
    write("mark-light.svg", svg(MARK_VIEWBOX, l_body, "rhizo mark (for light backgrounds)", l_defs))
    write("mark-mono.svg", svg(MARK_VIEWBOX, mono_mark(), "rhizo mark (one color)"))

    # Wordmarks
    vb = " ".join(map(str, WORD_VIEWBOX))
    write("wordmark-light.svg", svg(vb, wordmark(INK, LIGHT["indigo"]), "rhizo"))
    write("wordmark-dark.svg", svg(vb, wordmark(PAPER, DARK["indigo"]), "rhizo"))

    # Lockups: mark + wordmark, wordmark x-height centered on the seed.
    mw, mh = 120, 142
    for variant, (defs, body, ink, accent) in {
        "light": (l_defs, l_body, INK, LIGHT["indigo"]),
        "dark": (d_defs, d_body, PAPER, DARK["indigo"]),
    }.items():
        content = (
            nested(0, 0, mw, mh, MARK_VIEWBOX, body)
            + nested(150, 22, 312 * 0.95, 110 * 0.95, vb, wordmark(ink, accent))
        )
        write(f"logo-{variant}.svg", svg(f"0 0 {150 + 312 * 0.95:.0f} {mh}", content, "rhizo", defs))

    # Favicon: the mark only, adapting to the browser's color scheme.
    fav_defs = (
        "<style>.seed{fill:#14131f}.core{fill:#6366f1}"
        "@media (prefers-color-scheme: dark){.seed{fill:#f8f8fc}.core{fill:#818cf8}}</style>"
    )
    fav = []
    for name, d, _ in SHOOTS:
        fav.append(f'<path d="{d}" fill="none" stroke="{LIGHT[name]}" stroke-width="56" stroke-linecap="round"/>')
    for name, _, (cx, cy, _) in SHOOTS:
        fav.append(f'<circle cx="{cx}" cy="{cy}" r="62" fill="{LIGHT[name]}"/>')
    fav.append('<circle class="seed" cx="356" cy="512" r="104"/><circle class="core" cx="356" cy="512" r="38"/>')
    write("favicon.svg", svg("226 196 556 632", "\n  ".join(fav), "rhizo", fav_defs))

    # Social card for the repository (1280×640).
    icon = (OUT / "icon.svg").read_text()
    icon_inner = icon[icon.index(">", icon.index("<svg")) + 1 : icon.rindex("</svg>")]
    icon_inner = icon_inner.replace('id="', 'id="i-').replace("url(#", "url(#i-")
    chips = ["Mermaid", "D2", "Graphviz", "draw.io", "OPML", "FreeMind", "JSON Canvas", "SVG · PNG · PDF"]
    chip_svg, x, y = [], 560, 460
    for c in chips:
        w = 26 + len(c) * 9.4
        if x + w > 1200:
            x, y = 560, y + 48
        chip_svg.append(
            f'<rect x="{x}" y="{y}" width="{w:.0f}" height="38" rx="19" fill="#ffffff" fill-opacity="0.06" '
            f'stroke="#ffffff" stroke-opacity="0.12"/>'
            f'<text x="{x + w / 2:.0f}" y="{y + 25}" text-anchor="middle" font-size="17" fill="#c7c7d6">{c}</text>'
        )
        x += w + 10
    card = f"""<defs>
    <linearGradient id="card" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#1d1b30"/><stop offset="1" stop-color="#0b0b11"/>
    </linearGradient>
    <radialGradient id="cardglow" cx="0.25" cy="0.5" r="0.6">
      <stop offset="0" stop-color="#6366f1" stop-opacity="0.28"/><stop offset="1" stop-color="#6366f1" stop-opacity="0"/>
    </radialGradient>
  </defs>
  <rect width="1280" height="640" fill="url(#card)"/>
  <rect width="1280" height="640" fill="url(#cardglow)"/>
  {nested(60, 120, 400, 400, "0 0 1024 1024", icon_inner)}
  {nested(556, 150, 312 * 1.25, 110 * 1.25, vb, wordmark(PAPER, DARK["indigo"]))}
  <g font-family="Inter, 'SF Pro Display', 'Helvetica Neue', Helvetica, Arial, sans-serif">
    <text x="560" y="380" font-size="44" font-weight="600" fill="#f4f4f5">Think in graphs.</text>
    <text x="560" y="432" font-size="44" font-weight="600" fill="#a5b4fc">Ship in any format.</text>
    {"".join(chip_svg)}
  </g>"""
    write("social-card.svg", svg("0 0 1280 640", card, "rhizo: think in graphs, ship in any format", w=1280, h=640))


def brand_sheet():
    """Inlines the SVGs into the brand sheet page."""
    import re

    def inline(name):
        text = (OUT / name).read_text()
        text = re.sub(r"<title>.*?</title>", "", text, flags=re.S)
        return re.sub(r'\s(width|height)="\d+"', "", text, count=2)

    page = (OUT / "brand-sheet.template.html").read_text()
    for key, name in {
        "ICON": "icon.svg",
        "WORDMARK": "wordmark-dark.svg",
        "MARK_DARK": "mark-dark.svg",
        "MARK_LIGHT": "mark-light.svg",
        "MARK_MONO": "mark-mono.svg",
        "LOGO_LIGHT": "logo-light.svg",
        "LOGO_DARK": "logo-dark.svg",
        "SOCIAL": "social-card.svg",
    }.items():
        svg_text = inline(name)
        if key == "WORDMARK":
            # The hero wordmark follows the page's text color in both themes.
            svg_text = svg_text.replace(f'stroke="{PAPER}"', 'stroke="currentColor"')
        page = page.replace("{{" + key + "}}", svg_text)
    write("brand-sheet.html", page)


if __name__ == "__main__":
    main()
    brand_sheet()

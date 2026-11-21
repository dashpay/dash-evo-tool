#!/usr/bin/python3
"""Generate branded slides with an external font and a local browser, offline."""

import argparse
import base64
import hashlib
from pathlib import Path
import subprocess
import tempfile

SOURCE = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--font", type=Path, required=True)
    parser.add_argument("--browser", default="google-chrome")
    parser.add_argument(
        "--no-sandbox",
        action="store_true",
        help="Only for an isolated restricted desktop",
    )
    args = parser.parse_args()
    ROOT = args.output.resolve()
    ROOT.mkdir(parents=True, exist_ok=True)
    if any(ROOT.iterdir()):
        parser.error("Output must be empty; generated assets are never overwritten")
    logo = (SOURCE / "logo-dash-evo-full-white.svg").read_bytes()
    logo_uri = "data:image/svg+xml;base64," + base64.b64encode(logo).decode()
    font_uri = args.font.resolve().as_uri()
    if not args.font.is_file():
        parser.error("Supply an existing external font")
    for width, height in [(1920, 1080)]:
        vh = height * 1920 / width
        for kind in ["opening", "closing"]:

            def text(x, y, size, value, color="#ffffff", weight=500, spacing=0):
                return f'<text x="{x}" y="{y}" fill="{color}" font-size="{size}" font-weight="{weight}" letter-spacing="{spacing}">{value}</text>'

            content = [
                f'<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{width}" height="{height}" viewBox="0 0 1920 {vh}"><title>{kind.title()} — Create Tokens on Dash Evolution</title><desc>Dash Evolution token tutorial featuring Chad G. P. Taylor. Official unmodified white logo, CC BY 4.0, Dash, https://www.dash.org/brand-guidelines/</desc><defs><style>@font-face{{font-family:Montserrat;src:url({font_uri}) format("truetype");font-weight:100 900}}text{{font-family:Montserrat,sans-serif}}</style><linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop stop-color="#0b0f3b"/><stop offset="1" stop-color="#012060"/></linearGradient><radialGradient id="halo"><stop stop-color="#008de4" stop-opacity=".18"/><stop offset="1" stop-color="#008de4" stop-opacity="0"/></radialGradient><linearGradient id="plane" x1="0" y1="0" x2="1" y2="1"><stop stop-color="#008de4" stop-opacity=".19"/><stop offset="1" stop-color="#008de4" stop-opacity=".02"/></linearGradient></defs>',
                f'<rect width="1920" height="{vh}" fill="url(#bg)"/><ellipse cx="1660" cy="570" rx="700" ry="650" fill="url(#halo)"/>',
                '<g fill="none" stroke="#589edd" stroke-opacity=".13"><circle cx="1660" cy="550" r="460"/><circle cx="1660" cy="550" r="350"/><path d="M1250 550H1920M1660 90V1010" stroke-dasharray="3 12"/></g>',
                '<g transform="translate(1440 370)"><path d="M0 220 220 90 440 220 220 350Z" fill="url(#plane)" stroke="#008de4" stroke-opacity=".35"/><path d="M0 160 220 30 440 160 220 290Z" fill="url(#plane)" stroke="#008de4" stroke-opacity=".55"/><path d="M0 100 220 -30 440 100 220 230Z" fill="url(#plane)" stroke="#52b5f0" stroke-opacity=".75"/><path d="M220 -30V350" stroke="#6bc4ff" stroke-opacity=".27" stroke-dasharray="4 10"/><circle cx="220" cy="-30" r="6" fill="#62c3ff"/><circle cx="0" cy="100" r="4" fill="#008de4"/><circle cx="440" cy="100" r="4" fill="#008de4"/></g>',
                f'<image x="128" y="92" width="310" height="129" preserveAspectRatio="xMidYMid meet" xlink:href="{logo_uri}"/>',
                '<path d="M128 307H194" stroke="#008de4" stroke-width="5"/>',
            ]
            if kind == "opening":
                content += [
                    text(128, 448, 88, "Create Tokens", weight=700),
                    text(128, 554, 88, "on Dash Evolution", weight=700),
                    text(
                        132,
                        638,
                        33,
                        "From your first token to advanced controls",
                        "#b6cce5",
                        400,
                    ),
                    text(
                        132,
                        760,
                        29,
                        "with Chad G. P. Taylor &amp; Ewa Dashwood",
                        "#ffffff",
                        500,
                    ),
                    text(
                        132,
                        830,
                        27,
                        "This presentation was generated entirely by AI",
                        "#ffffff",
                        500,
                    ),
                    text(132, 870, 27, "and may contain mistakes.", "#ffffff", 500),
                ]
            else:
                content += [
                    text(128, 440, 88, "Your token. Your rules.", weight=700),
                    text(128, 546, 76, "Know what they allow.", weight=700),
                    text(
                        132,
                        638,
                        33,
                        "Create carefully. Explain clearly.",
                        "#b6cce5",
                        400,
                    ),
                    text(
                        132,
                        812,
                        29,
                        "Chad G. P. Taylor &amp; Ewa Dashwood",
                        "#ffffff",
                        500,
                    ),
                ]
            content += [
                f'<path d="M128 {vh - 136}H1792" stroke="#86aad0" stroke-opacity=".22"/>',
                text(
                    128,
                    vh - 78,
                    20,
                    "Dash Evo Tool · Testnet tutorial · AI-generated narration",
                    "#9eb8d7",
                    400,
                ),
                "</svg>",
            ]
            stem = f"{kind}-{width}x{height}"
            svg = ROOT / f"{stem}.svg"
            svg.write_text("".join(content), encoding="utf-8")
            html = ROOT / f".{stem}.html"
            html.write_text(
                f'<html><head><meta charset="utf-8"><style>html,body{{margin:0;width:{width}px;height:{height}px;overflow:hidden}}</style></head><body>{svg.read_text()}</body></html>'
            )
            with tempfile.TemporaryDirectory(
                prefix="dash-slide-chrome-",
            ) as profile:
                subprocess.run(
                    [
                        args.browser,
                        *(["--no-sandbox"] if args.no_sandbox else []),
                        "--headless",
                        "--disable-gpu",
                        "--hide-scrollbars",
                        "--force-device-scale-factor=1",
                        "--no-pdf-header-footer",
                        "--virtual-time-budget=1200",
                        f"--user-data-dir={profile}",
                        f"--window-size={width},{height}",
                        f"--screenshot={ROOT / (stem + '.png')}",
                        html.as_uri(),
                    ],
                    check=True,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            html.unlink()
            print(stem)
    print("Official logo SHA-256:", hashlib.sha256(logo).hexdigest())

    # Editorial slides are plain source SVGs; fonts remain external until rasterization.
    for sid in ("01", "02", "50"):
        name = f"scene-{sid}"
        svg = (SOURCE / "slides" / f"{name}.svg").read_text()
        font_css = f"<style>@font-face{{font-family:Montserrat;src:url({font_uri})}}text{{font-family:Montserrat,sans-serif}}</style>"
        svg = svg.replace("</svg>", font_css + "</svg>")
        (ROOT / f"{name}.svg").write_text(svg)
        html = ROOT / f".{name}.html"
        html.write_text(
            "<html><style>html,body{margin:0;width:1920px;height:1080px;overflow:hidden}</style>"
            + svg
            + "</html>"
        )
        with tempfile.TemporaryDirectory(prefix="tutorial-slide-") as profile:
            subprocess.run(
                [
                    args.browser,
                    *(["--no-sandbox"] if args.no_sandbox else []),
                    "--headless",
                    "--disable-gpu",
                    "--hide-scrollbars",
                    "--force-device-scale-factor=1",
                    "--virtual-time-budget=1200",
                    f"--user-data-dir={profile}",
                    "--window-size=1920,1080",
                    f"--screenshot={ROOT / (name + '.png')}",
                    html.as_uri(),
                ],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        html.unlink()


if __name__ == "__main__":
    main()

#!/usr/bin/python3
"""Render the eight pinned tutorial examples, offline; not a Platform evaluator."""

import argparse
import base64
import hashlib
import io
import json
import math
import os
from pathlib import Path
import subprocess
import tempfile

TUTORIAL = Path(__file__).resolve().parents[2]
ROOT = TUTORIAL / "local/emission-examples"
os.environ.setdefault(
    "MPLCONFIGDIR", str(Path(tempfile.gettempdir()) / "tutorial-matplotlib")
)

import matplotlib  # noqa: E402

matplotlib.use("Agg")
from matplotlib import font_manager, pyplot as plt  # noqa: E402
from matplotlib.patches import Rectangle  # noqa: E402

DET_SHA = "32f8ede362bd0b4e02b641f022ad6d88b0087a48"
PLATFORM_SHA = "575d961fe0c54bd7e01a3cf3823494e58c6c4fdb"
EVALUATOR = (
    f"https://github.com/dashpay/platform/blob/{PLATFORM_SHA}/packages/rs-dpp/"
    "src/data_contract/associated_token/token_perpetual_distribution/"
    "distribution_function/"
)
FONT = None
BROWSER = "google-chrome"
BROWSER_OPTIONS = []
LOGO = TUTORIAL / "assets/logo-dash-evo-full-white.svg"
BLUE, INK, MUTED, PALE = "#008de4", "#102e50", "#527087", "#edf6fd"


def recipes():
    """Exact example parameters; s=None means the GUI field stays blank."""
    return [
        dict(
            name="FixedAmount",
            title="Fixed amount",
            scene="35",
            parameters={"amount": 100},
            formula="f(k) = 100",
            card=["Amount per interval = 100", "No min / max fields"],
            note="The same amount at every index.",
            eval_line=34,
            valid_line=23,
        ),
        dict(
            name="StepDecreasing",
            title="Step decreasing",
            scene="36",
            parameters=dict(
                step_count=2,
                decrease_numerator=1,
                decrease_denominator=2,
                s=None,
                initial=100,
                minimum=10,
                maximum_interval_count=3,
                trailing=10,
            ),
            formula="100 → 50 → 25 → 12 → 10",
            card=[
                "Step Count = 2",
                "Decrease fraction = 1 / 2 (removed)",
                "Initial = 100; minimum = 10",
                "Maximum Interval Count = 3",
                "Trailing amount = 10",
                "Start Period Offset = blank",
            ],
            note="Integer division at each decrease; trailing after era 3.",
            eval_line=63,
            valid_line=71,
        ),
        dict(
            name="Stepwise",
            title="Stepwise",
            scene="37",
            parameters={"steps": [[0, 10], [3, 5], [5, 0]]},
            formula="0 ≤ k < 3: 10;   3 ≤ k < 5: 5;   k ≥ 5: 0",
            card=[
                "Start Step → Amount",
                "0 → 10",
                "3 → 5",
                "5 → 0",
                "No min / max fields",
            ],
            note="Use the amount at the latest threshold reached.",
            eval_line=113,
            valid_line=179,
        ),
        dict(
            name="Linear",
            title="Linear",
            scene="38",
            parameters=dict(a=2, d=1, s=None, b=10, minimum=0, maximum=20),
            formula="f(k) = clamp(2k + 10, 0, 20)",
            card=[
                "a = 2; d = 1; b = 10",
                "Start Step s = blank",
                "Min Emission = 0",
                "Max Emission = 20",
            ],
            note="The maximum limits each interval, not total supply.",
            eval_line=125,
            valid_line=194,
        ),
        dict(
            name="Polynomial",
            title="Polynomial",
            scene="39",
            parameters=dict(
                a=1, m=2, n=1, d=1, s=None, o=1, b=0, minimum=0, maximum=100
            ),
            formula="f(k) = clamp((k + 1)², 0, 100)",
            card=[
                "a = 1; m = 2; n = 1; d = 1",
                "o = 1; b = 0",
                "Start Step s = blank",
                "Min Emission = 0",
                "Max Emission = 100",
            ],
            note="The offset is inside the power; this example uses squares.",
            eval_line=195,
            valid_line=302,
        ),
        dict(
            name="Exponential",
            title="Exponential",
            scene="40",
            parameters=dict(
                a=10, m=1, n=1, d=1, s=None, o=0, b=0, minimum=0, maximum=100
            ),
            formula="f(k) = floor(clamp(10 × exp(k), 0, 100))",
            card=[
                "a = 10; m = 1; n = 1; d = 1",
                "o = 0; b = 0",
                "Start Step s = blank",
                "Min Emission = 0",
                "Max Emission = 100",
            ],
            note="exp uses base e. The maximum is reached at k = 3.",
            eval_line=302,
            valid_line=478,
        ),
        dict(
            name="Logarithmic",
            title="Logarithmic",
            scene="41",
            parameters=dict(
                a=10, d=1, m=1, n=1, s=None, o=1, b=10, minimum=0, maximum=100
            ),
            formula="f(k) = clamp(floor(10 × ln(k + 1)) + 10, 0, 100)",
            card=[
                "a = 10; d = 1; m = 1; n = 1",
                "o = 1; b = 10",
                "Start Step s = blank",
                "Min Emission = 0",
                "Max Emission = 100",
            ],
            note="Natural log; round down before adding b.",
            eval_line=370,
            valid_line=668,
        ),
        dict(
            name="InvertedLogarithmic",
            title="Inverted logarithmic",
            scene="42",
            parameters=dict(
                a=10, d=1, m=1, n=1, s=None, o=1, b=40, minimum=0, maximum=100
            ),
            formula="f(k) = clamp(floor(10 × ln(1 / (k + 1))) + 40, 0, 100)",
            card=[
                "a = 10; d = 1; m = 1; n = 1",
                "o = 1; b = 40",
                "Start Step s = blank",
                "Min Emission = 0",
                "Max Emission = 100",
            ],
            note="Negative floor matters: floor(−6.93) + 40 = 33.",
            eval_line=489,
            valid_line=829,
        ),
    ]


def evaluate(name, p, k):
    """Calculate only these small, audited recipes, with their rounding rules."""
    if name == "FixedAmount":
        return p["amount"]
    if name == "StepDecreasing":
        era = k // p["step_count"]
        if era > p["maximum_interval_count"]:
            return p["trailing"]
        amount = p["initial"]
        for _ in range(era):
            amount = (
                amount
                * (p["decrease_denominator"] - p["decrease_numerator"])
                // p["decrease_denominator"]
            )
        return max(p["minimum"], amount)
    if name == "Stepwise":
        return next(amount for start, amount in reversed(p["steps"]) if start <= k)
    if name == "Linear":
        amount = p["a"] * k // p["d"] + p["b"]
    elif name == "Polynomial":
        amount = p["a"] * int((k + p["o"]) ** (p["m"] / p["n"])) // p["d"] + p["b"]
    elif name == "Exponential":
        amount = p["a"] * math.exp(p["m"] * (k + p["o"]) / p["n"]) / p["d"] + p["b"]
    elif name in ("Logarithmic", "InvertedLogarithmic"):
        argument = p["m"] * (k + p["o"]) / p["n"]
        if name == "InvertedLogarithmic":
            argument = 1 / argument
        amount = math.floor(p["a"] * math.log(argument) / p["d"]) + p["b"]
    else:
        raise ValueError(f"Unknown example: {name}")
    return int(max(p["minimum"], min(p["maximum"], amount)))


def render(example, number):
    values = example["values_base_units"]
    fig = plt.figure(figsize=(19.2, 10.8), dpi=100, facecolor="white")
    fig.add_artist(Rectangle((0, 0.84), 1, 0.16, transform=fig.transFigure, color=INK))
    fig.text(
        0.045, 0.94, f"EMISSION EXAMPLE {number:02} / 08", color="#a9d9f8", fontsize=13
    )
    fig.text(0.045, 0.88, example["title"], color="white", fontsize=31, weight="bold")
    fig.text(0.045, 0.79, example["formula"], color=INK, fontsize=20)
    ax = fig.add_axes((0.075, 0.365, 0.54, 0.36))
    ax.plot(range(11), values, color=BLUE, linewidth=2, linestyle=":", zorder=2)
    ax.scatter(range(1, 11), values[1:], color=BLUE, s=66, zorder=3)
    ax.scatter(
        [0],
        values[:1],
        facecolor="white",
        edgecolor=MUTED,
        s=100,
        linewidth=2,
        zorder=4,
    )
    ax.set(
        xlim=(-0.35, 10.35),
        ylim=(-max(values) * 0.05 - 0.5, max(values) * 1.2 + 1),
        xticks=range(11),
        xlabel="Relative period index k",
        ylabel="f(k) · base units",
    )
    ax.grid(axis="y", color="#dce9f3", zorder=0)
    ax.spines[["top", "right"]].set_visible(False)
    ax.spines[["left", "bottom"]].set_color("#bed2e2")
    ax.tick_params(colors=MUTED)
    fig.add_artist(
        Rectangle(
            (0.66, 0.35), 0.30, 0.395, transform=fig.transFigure, color=PALE, zorder=0
        )
    )
    fig.text(0.68, 0.71, "PARAMETERS", fontsize=14, weight="bold", color=INK)
    for index, line in enumerate(example["card"]):
        fig.text(0.68, 0.666 - index * 0.041, line, fontsize=14, color=INK)
    fig.text(
        0.68,
        0.373,
        f"Range shown: {min(values)}–{max(values)} base units",
        fontsize=13,
        color=MUTED,
    )
    fig.text(0.045, 0.302, example["note"], fontsize=16, color=INK)
    table_ax = fig.add_axes((0.045, 0.145, 0.915, 0.132))
    table_ax.set_axis_off()
    rows = [
        ["k"] + list(range(11)),
        ["Base units"] + values,
        ["Tokens (2 dp)"] + example["values_tokens_decimals_2"],
    ]
    table = table_ax.table(
        cellText=rows,
        cellLoc="center",
        loc="center",
        colWidths=[0.14] + [0.86 / 11] * 11,
    )
    table.auto_set_font_size(False)
    table.set_fontsize(13)
    for (row, column), cell in table.get_celld().items():
        cell.set_height(0.32)
        cell.set_edgecolor("#dce9f3")
        cell.set_text_props(color=INK)
        cell.set_facecolor(PALE if row == 0 or column == 0 else "white")
    total = example["sum_f1_to_f5_base_units"]
    fig.text(
        0.045,
        0.11,
        f"f(1) + … + f(5) = {total} base units = {total / 100:.2f} tokens at decimals = 2",
        fontsize=16,
        weight="bold",
        color=INK,
    )
    fig.text(
        0.045,
        0.071,
        "Hollow marker: f(0) is illustrative, not Initial Supply. Lines only guide the eye.",
        fontsize=12,
        color=MUTED,
    )
    fig.text(
        0.045,
        0.039,
        "Illustration; no claim submitted · Period indices do not promise a wall-clock claim time.",
        fontsize=12,
        color=MUTED,
    )
    output = io.StringIO()
    fig.savefig(output, format="svg", metadata={"Date": None})
    plt.close(fig)
    svg = output.getvalue().replace(
        'width="1382.4pt" height="777.6pt"', 'width="1920" height="1080"'
    )
    font_uri = FONT.as_uri()
    logo_uri = base64.b64encode(LOGO.read_bytes()).decode()
    embed = (
        "<defs><style>@font-face{font-family:Montserrat;src:url("
        + font_uri
        + ') format("truetype");font-weight:100 900}</style></defs>'
        '<image x="1180" y="14" width="145" height="85" preserveAspectRatio="xMidYMid meet" '
        'xlink:href="data:image/svg+xml;base64,' + logo_uri + '"/>'
    )
    svg = svg.replace("</svg>", embed + "</svg>")
    stem = f"{number:02}-{example['name'].lower()}"
    (ROOT / f"{stem}.svg").write_text(svg, encoding="utf-8")
    # SVG references the operator-supplied font; raster exports need no font files.
    with tempfile.TemporaryDirectory(prefix="emission-render-") as scratch:
        scratch = Path(scratch)
        html = scratch / "image.html"
        html.write_text(
            '<html><head><meta charset="utf-8"><style>body{margin:0}</style></head><body>'
            + svg
            + "</body></html>",
            encoding="utf-8",
        )
        subprocess.run(
            [
                BROWSER,
                *BROWSER_OPTIONS,
                "--headless",
                "--disable-gpu",
                "--hide-scrollbars",
                "--force-device-scale-factor=1",
                "--virtual-time-budget=1200",
                "--window-size=1920,1080",
                f"--user-data-dir={scratch / 'profile'}",
                f"--screenshot={ROOT / (stem + '.png')}",
                html.as_uri(),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    return {"svg": f"{stem}.svg", "png": f"{stem}.png"}


def main():
    global ROOT, FONT, BROWSER, BROWSER_OPTIONS
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
    ROOT, FONT, BROWSER = args.output.resolve(), args.font.resolve(), args.browser
    BROWSER_OPTIONS = ["--no-sandbox"] if args.no_sandbox else []
    ROOT.mkdir(parents=True, exist_ok=True)
    if any(ROOT.iterdir()):
        parser.error("Output must be empty; generated assets are never overwritten")
    font_manager.fontManager.addfont(FONT)
    plt.rcParams.update(
        {
            "font.family": font_manager.FontProperties(fname=FONT).get_name(),
            "font.size": 14,
            "svg.fonttype": "none",
            "svg.hashsalt": "emission-examples",
        }
    )
    examples = recipes()
    for number, example in enumerate(examples, 1):
        values = [
            evaluate(example["name"], example["parameters"], k) for k in range(11)
        ]
        example.update(
            values_base_units=values,
            values_tokens_decimals_2=[f"{value / 100:.2f}" for value in values],
            sum_f1_to_f5_base_units=sum(values[1:6]),
            observed_minimum=min(values),
            observed_maximum=max(values),
            evaluator_url=EVALUATOR + f"evaluate.rs#L{example['eval_line']}",
            validator_url=EVALUATOR + f"validation.rs#L{example['valid_line']}",
        )
        example["files"] = render(example, number)
    inventory = TUTORIAL / "scenario/dialogue.json"
    record = dict(
        schema_version=1,
        det_sha=DET_SHA,
        platform_sha=PLATFORM_SHA,
        inventory="scenario/dialogue.json",
        inventory_sha256=hashlib.sha256(inventory.read_bytes()).hexdigest(),
        generator_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        logo_sha256=hashlib.sha256(LOGO.read_bytes()).hexdigest(),
        calculation="Static Python calculation of eight audited recipes; no Rust or network executed.",
        k_definition="k=x-r, relative to registration period index r; s remains blank.",
        decimals=2,
        k=list(range(11)),
        caveats=[
            "f(0) is illustrative, not an additional Initial Supply payout.",
            "Five-period sum excludes k=0 and includes k=1 through k=5.",
            "Period indices are not promised wall-clock claim times.",
            "No claim submitted. Examples are separate alternatives, not concurrent functions.",
            "Emission bounds apply per interval; they are not Max Supply.",
        ],
        examples=examples,
    )
    (ROOT / "values.json").write_text(
        json.dumps(record, indent=2) + "\n", encoding="utf-8"
    )
    print(f"Rendered {len(examples)} SVG/PNG pairs and values.json.")


if __name__ == "__main__":
    main()

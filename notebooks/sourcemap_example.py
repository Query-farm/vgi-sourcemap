"""Presentation for the Source Maps notebook; analytical results remain in SQL."""

import matplotlib.pyplot as plt
from matplotlib.ticker import MaxNLocator, ScalarFormatter
from vgi_notebooks import finish

SOURCE = "Authored map and fifteen synthetic frames · VGI Source Maps v0.4.0 · No deployed telemetry"


def plot_gaps(gaps, MAX_COLUMN_GAP):
    fig, ax = plt.subplots(figsize=(12, 7.5))
    colors = ["#267358" if s == "Included by policy" else "#c57b40" for s in gaps.review_state]
    ax.hlines(range(len(gaps)), 0, gaps.column_gap, color="#d6dfd8", linewidth=2)
    ax.scatter(gaps.column_gap, range(len(gaps)), c=colors, s=85, zorder=3)
    for y, gap in enumerate(gaps.column_gap):
        ax.annotate(
            str(int(gap)),
            (gap, y),
            xytext=(8, 0),
            textcoords="offset points",
            va="center",
            fontsize=10,
            bbox={"facecolor": "white", "edgecolor": "none", "pad": 1},
        )
    ax.set_yticks(range(len(gaps)), gaps.frame_id)
    ax.invert_yaxis()
    ax.set_xscale("symlog", linthresh=1)
    ax.set_xticks([0, 1, 4, 10, 30, 100])
    ax.xaxis.set_major_formatter(ScalarFormatter())
    ax.set_xlim(-0.1, 180)
    ax.axvline(MAX_COLUMN_GAP, color="#456fba", linestyle="--", linewidth=1.6, zorder=1)
    ax.text(
        0.99,
        1.02,
        f"Policy maximum: {MAX_COLUMN_GAP} columns",
        transform=ax.transAxes,
        ha="right",
        color="#456fba",
    )
    ax.set_xlabel("Generated-column gap · symlog scale (linear from 0 to 1)")
    ax.set_ylabel("Same-line frame ID")
    ax.grid(axis="y", visible=False)
    finish(
        fig,
        "Nearest does not imply nearby",
        "Same-line matches only · Green: included by policy · Amber: larger gap",
        SOURCE,
        name=f"mapping-gaps-{MAX_COLUMN_GAP}",
    )


def plot_counts(counts, MAX_COLUMN_GAP):
    fig, ax = plt.subplots(figsize=(12, 7))
    bars = ax.barh(
        counts.state,
        counts.frames,
        color=["#267358", "#c57b40", "#456fba", "#a3aea6", "#718177"],
        height=0.55,
    )
    ax.bar_label(bars, padding=6, fontsize=12)
    ax.invert_yaxis()
    ax.set_xlim(0, max(1, int(counts.frames.sum())))
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.set_xlabel("Synthetic stack-frame rows · Full input denominator")
    ax.grid(axis="y", visible=False)
    finish(
        fig,
        "A reporting policy changes coverage",
        f"Maximum gap: {MAX_COLUMN_GAP} columns · All frames retained in the audit",
        SOURCE,
        name=f"frame-accounting-{MAX_COLUMN_GAP}",
    )

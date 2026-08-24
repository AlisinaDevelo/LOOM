#!/usr/bin/env python3
"""Deterministic checks for LOOM's desktop accessibility contract.

This is a lightweight guard for the webview contract, not a replacement for a
manual VoiceOver pass or a user study. It keeps keyboard, live-region, focus,
reduced-motion, and the selected AA text-color budget from regressing silently.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
APP = (ROOT / "src" / "App.tsx").read_text()
CSS = (ROOT / "src" / "App.css").read_text()


def luminance(color: str) -> float:
    channels = [int(color[index : index + 2], 16) / 255 for index in (1, 3, 5)]
    linear = [channel / 12.92 if channel <= 0.04045 else ((channel + 0.055) / 1.055) ** 2.4 for channel in channels]
    return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]


def contrast(foreground: str, background: str) -> float:
    light, dark = sorted((luminance(foreground), luminance(background)), reverse=True)
    return (light + 0.05) / (dark + 0.05)


def css_color(variable: str) -> str:
    match = re.search(rf"{re.escape(variable)}:\s*(#[0-9a-fA-F]{{6}})", CSS)
    if not match:
        raise AssertionError(f"missing CSS color token {variable}")
    return match.group(1)


class AccessibilityContractTests(unittest.TestCase):
    def test_keyboard_and_focus_contract_is_present(self) -> None:
        for marker in (
            'className="skip-link"',
            'id="main-content"',
            'aria-keyshortcuts="Control+K Meta+K"',
            'focusResultsAfterSearchRef',
            'aria-atomic="true"',
        ):
            self.assertIn(marker, APP)

    def test_evidence_explains_match_and_exposes_named_viewer(self) -> None:
        for marker in (
            'className="match-reason"',
            'Why this matched',
            'role="region"',
            'role="img"',
            'aria-live="polite"',
        ):
            self.assertIn(marker, APP)

    def test_focus_and_reduced_motion_styles_are_present(self) -> None:
        self.assertIn(":where(a, button, input, select, textarea):focus-visible", CSS)
        self.assertIn("@media (prefers-reduced-motion: reduce)", CSS)
        self.assertIn(".result-card:hover { transform: none; }", CSS)

    def test_primary_dark_theme_text_tokens_meet_wcag_aa(self) -> None:
        background = css_color("--canvas")
        for variable in ("--text", "--muted", "--faint", "--thread"):
            self.assertGreaterEqual(
                contrast(css_color(variable), background),
                4.5,
                f"{variable} must meet 4.5:1 against {background}",
            )


if __name__ == "__main__":
    unittest.main()

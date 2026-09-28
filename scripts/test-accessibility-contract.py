#!/usr/bin/env python3
"""Deterministic checks for LOOM's desktop accessibility contract.

This is a lightweight guard for the webview contract, not a replacement for a
manual VoiceOver pass or a user study. It keeps keyboard, live-region, focus,
reduced-motion, and the selected AA text-color budget from regressing silently.
"""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
APP = (ROOT / "src" / "App.tsx").read_text()
CSS = (ROOT / "src" / "App.css").read_text()
INDEX = (ROOT / "index.html").read_text()
TAURI = json.loads((ROOT / "src-tauri" / "tauri.conf.json").read_text())

# Backgrounds that text and controls sit on: canvas, panels, sidebar, and scope rows.
BACKGROUNDS = ("--canvas", "--panel", "--panel-raised")
LITERAL_BACKGROUNDS = ("#141612", "#151814", "#11130f")


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

    def test_text_tokens_meet_aa_on_every_panel_background(self) -> None:
        backgrounds = [css_color(token) for token in BACKGROUNDS] + list(LITERAL_BACKGROUNDS)
        for variable in ("--text", "--muted", "--faint", "--thread", "--warm"):
            for background in backgrounds:
                self.assertGreaterEqual(
                    contrast(css_color(variable), background),
                    4.5,
                    f"{variable} must meet 4.5:1 against {background}",
                )

    def test_control_borders_and_focus_ring_meet_non_text_contrast(self) -> None:
        backgrounds = [css_color(token) for token in BACKGROUNDS] + list(LITERAL_BACKGROUNDS)
        for variable in ("--control-border", "--thread"):
            for background in backgrounds:
                self.assertGreaterEqual(
                    contrast(css_color(variable), background),
                    3.0,
                    f"{variable} must meet 3:1 against {background} (WCAG 1.4.11)",
                )
        for selector in (".search-form {", ".capture-context-fields input", ".viewer-close, .viewer-control {"):
            block = CSS[CSS.index(selector) :]
            block = block[: block.index("}")]
            self.assertIn("var(--control-border)", block, selector)

    def test_desktop_zoom_and_reflow_are_allowed(self) -> None:
        window = TAURI["app"]["windows"][0]
        self.assertTrue(window.get("zoomHotkeysEnabled"), "webview zoom must be enabled (WCAG 1.4.4)")
        self.assertNotIn("user-scalable=no", INDEX)
        self.assertNotIn("maximum-scale", INDEX)
        self.assertIsNone(re.search(r"(?m)^body\s*\{[^}]*min-width", CSS), "body min-width blocks reflow")
        self.assertIn("@media (max-width: 640px)", CSS)

    def test_pointer_targets_have_a_minimum_size(self) -> None:
        self.assertIn(':where(button, input:not([type="checkbox"]):not([type="radio"]), select)', CSS)
        self.assertIn("min-height: 24px;", CSS)
        self.assertIn(":where(button) { min-width: 24px; }", CSS)

    def test_evidence_viewer_returns_focus_and_closes_on_escape(self) -> None:
        for marker in ("evidenceTriggerRef", 'event.key === "Escape"', 'aria-keyshortcuts="Escape"'):
            self.assertIn(marker, APP)


if __name__ == "__main__":
    unittest.main()

# Accessibility

Target: WCAG 2.2 AA for the desktop webview's primary flow (search, results, evidence viewer,
library controls). Roadmap `0208`.

## What is checked automatically

`src/accessibility.test.tsx` runs in `npm test`:

- **axe-core** with the WCAG 2.0/2.1/2.2 A and AA rule sets against the empty state, a results
  list, and an open evidence viewer. jsdom has no layout engine, so axe's colour-contrast rule is
  replaced by the token checks below.
- **Focus order.** The skip link comes first, search precedes results, and no positive `tabindex`
  reorders the page.
- **Focus return.** Closing the evidence viewer, with the Close button or `Escape`, returns focus
  to the control that opened it (2.4.3).
- **Announcements.** Search progress and outcomes go through one polite, atomic live region; there
  are no assertive regions.

`scripts/test-accessibility-contract.py` runs in the device and CI contract checks:

| Check | WCAG |
| --- | --- |
| Text tokens reach 4.5:1 on the canvas, panels, sidebar, and scope rows | 1.4.3 |
| Control borders (`--control-border`) and the focus ring (`--thread`) reach 3:1 on the same backgrounds | 1.4.11 |
| Desktop zoom hotkeys are enabled and the viewport does not disable scaling | 1.4.4 |
| No `body` minimum width, and a 640px breakpoint stacks the sidebar | 1.4.10 |
| Buttons and inputs are at least 24px tall, buttons at least 24px wide | 2.5.8 |
| Visible focus style, reduced-motion media query, skip link, `Ctrl/Cmd+K` search shortcut | 2.4.7, 2.3.3, 2.4.1, 2.1.4 |
| Evidence viewer closes on `Escape` and exposes `aria-keyshortcuts` | 2.1.1 |

Reflow was also checked by hand in a browser at 320 CSS px: the sidebar stacks above the
workspace and the page has no horizontal scrolling.

## Not yet covered

- A manual VoiceOver pass in the packaged macOS app.
- Usability sessions showing that people understand why a result matched. That needs participants
  and belongs to the design-partner alpha (`0306`).
- Light mode and Windows high-contrast mode; the app ships a single dark theme.

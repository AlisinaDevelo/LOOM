import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import axe from "axe-core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import App from "./App";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const invokeMock = vi.mocked(invoke);

const readyStats = { source_roots: 1, artifacts: 1, versions: 1, passages: 1, indexed_bytes: 82 };

const hit = {
  rank: 1,
  score: 0.92,
  artifact_id: "11111111-1111-4111-8111-111111111111",
  version_id: "22222222-2222-4222-8222-222222222222",
  passage_id: "33333333-3333-4333-8333-333333333333",
  title: "isolation.md",
  media_type: "text/markdown",
  source_uri: "/Users/test/notes/isolation.md",
  content_hash: "blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  excerpt: {
    segments: [
      { text: "Serializable isolation prevents ", highlighted: false },
      { text: "retry anomalies", highlighted: true },
    ],
  },
  anchor: { kind: "text" as const, char_start: 31, char_end: 46, line_start: 2, line_end: 2 },
  confidence_state: "confirmed" as const,
  match_reason: "SQLite FTS5 BM25 over the active source passage",
};

const evidenceView = {
  artifact_id: hit.artifact_id,
  version_id: hit.version_id,
  passage_id: hit.passage_id,
  title: hit.title,
  media_type: hit.media_type,
  source_uri: hit.source_uri,
  content_hash: hit.content_hash,
  passage_text: "Serializable isolation prevents retry anomalies",
  anchor: hit.anchor,
  page_count: null,
  extractor_id: "loom.text",
  extractor_version: "0.1.0",
  extraction_metadata: {},
};

function mockLibrary() {
  invokeMock.mockImplementation(async (command) => {
    if (command === "reconcile_approved_roots") return {};
    if (command === "list_source_roots") return [];
    if (command === "library_stats") return readyStats;
    if (command === "capture_status") {
      return { paused: false, excluded_apps: [], capture_root: "/tmp/loom-captures", policy_error: null };
    }
    if (command === "search") return [hit];
    if (command === "resolve_evidence") return evidenceView;
    if (command === "list_relationships") return [];
    throw new Error(`unexpected command: ${command}`);
  });
}

/** Runs axe-core's WCAG 2.x A/AA rules. jsdom has no layout engine, so the color-contrast rule is
 * covered separately by scripts/test-accessibility-contract.py. */
async function expectNoWcagViolations(container: HTMLElement) {
  const results = await axe.run(container, {
    runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"] },
    rules: { "color-contrast": { enabled: false } },
  });
  const summary = results.violations.map(
    (violation) => `${violation.id}: ${violation.nodes.map((node) => node.target.join(" ")).join(", ")}`,
  );
  expect(summary).toEqual([]);
}

async function searchAndOpenEvidence() {
  const input = screen.getByRole("textbox", { name: "Search your local sources" });
  fireEvent.change(input, { target: { value: "retry anomalies" } });
  fireEvent.submit(screen.getByRole("search"));
  const trigger = await screen.findByRole("button", { name: "View evidence" });
  trigger.focus();
  fireEvent.click(trigger);
  await screen.findByRole("heading", { name: hit.title, level: 2 });
  return trigger;
}

describe("accessibility of the primary search workflow", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    mockLibrary();
  });

  afterEach(() => {
    cleanup();
  });

  it("has no automated WCAG A/AA violations in the empty, results, and evidence states", async () => {
    const { container } = render(<App />);
    await screen.findByRole("heading", { name: "Recover the exact thing." });
    await expectNoWcagViolations(container);

    const input = screen.getByRole("textbox", { name: "Search your local sources" });
    fireEvent.change(input, { target: { value: "retry anomalies" } });
    fireEvent.submit(screen.getByRole("search"));
    await screen.findByRole("heading", { name: "Recovered sources" });
    await expectNoWcagViolations(container);

    fireEvent.click(screen.getByRole("button", { name: "View evidence" }));
    await screen.findByRole("heading", { name: hit.title, level: 2 });
    await expectNoWcagViolations(container);
  });

  it("keeps a logical focus order: skip link, then search, then results", async () => {
    render(<App />);
    const skip = screen.getByRole("link", { name: "Skip to search and results" });
    expect(skip).toHaveAttribute("href", "#main-content");
    const main = document.getElementById("main-content");
    expect(main).toHaveAttribute("tabindex", "-1");

    const focusable = Array.from(
      document.querySelectorAll<HTMLElement>("a[href], button:not([disabled]), input:not([disabled])"),
    );
    expect(focusable[0]).toBe(skip);
    const search = screen.getByRole("textbox", { name: "Search your local sources" });
    expect(focusable.indexOf(search)).toBeGreaterThan(0);
    // No positive tabindex reorders the page.
    expect(document.querySelectorAll("[tabindex]:not([tabindex='-1']):not([tabindex='0'])")).toHaveLength(0);
  });

  it("returns focus to the evidence trigger when the viewer closes", async () => {
    render(<App />);
    const trigger = await searchAndOpenEvidence();
    expect(screen.getByRole("heading", { name: hit.title, level: 2 })).toHaveFocus();

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(trigger).toHaveFocus());
    expect(screen.queryByRole("heading", { name: hit.title, level: 2 })).not.toBeInTheDocument();
  });

  it("closes the evidence viewer with Escape and announces the shortcut", async () => {
    render(<App />);
    const trigger = await searchAndOpenEvidence();
    expect(screen.getByRole("button", { name: "Close" })).toHaveAttribute("aria-keyshortcuts", "Escape");

    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(trigger).toHaveFocus());
    expect(screen.queryByRole("region", { name: hit.title })).not.toBeInTheDocument();
  });

  it("announces search progress and outcomes through one polite live region", async () => {
    render(<App />);
    const regions = document.querySelectorAll("[aria-live]");
    const notice = screen.getAllByRole("status").find((element) => element.classList.contains("notice"));
    expect(notice).toHaveAttribute("aria-live", "polite");
    expect(notice).toHaveAttribute("aria-atomic", "true");
    expect(Array.from(regions).filter((region) => region.getAttribute("aria-live") === "assertive")).toHaveLength(0);

    fireEvent.change(screen.getByRole("textbox", { name: "Search your local sources" }), {
      target: { value: "retry anomalies" },
    });
    fireEvent.submit(screen.getByRole("search"));
    await screen.findByRole("heading", { name: "Recovered sources" });
    expect(notice?.textContent).not.toBe("");
  });
});

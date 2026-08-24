import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import App from "./App";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const invokeMock = vi.mocked(invoke);

const emptyStats = {
  source_roots: 0,
  artifacts: 0,
  versions: 0,
  passages: 0,
  indexed_bytes: 0,
};

const readyStats = {
  source_roots: 1,
  artifacts: 1,
  versions: 1,
  passages: 1,
  indexed_bytes: 82,
};

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
  anchor: {
    kind: "text" as const,
    char_start: 31,
    char_end: 46,
    line_start: 2,
    line_end: 2,
  },
  match_reason: "SQLite FTS5 BM25 over the active source passage",
};

const pdfHit = {
  ...hit,
  title: "paper.pdf",
  media_type: "application/pdf",
  anchor: {
    kind: "pdf_page" as const,
    page: 2,
    char_start: 6,
    char_end: 25,
    line_start: 1,
    line_end: 1,
  },
};

const imageHit = {
  ...hit,
  title: "screenshot.png",
  media_type: "image/png",
  anchor: {
    kind: "image_region" as const,
    char_start: 0,
    char_end: 18,
    line_start: 1,
    line_end: 1,
    x: 100,
    y: 50,
    width: 200,
    height: 100,
    image_width: 1200,
    image_height: 600,
    orientation: 1,
    scale_milli: 1000,
    confidence_milli: 990,
  },
};

describe("desktop truth path", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("exposes an accessible empty state and keyboard-search form", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "reconcile_approved_roots") return {};
      if (command === "list_source_roots") return [];
      if (command === "library_stats") return emptyStats;
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);

    expect(await screen.findByText("Start with a folder you intentionally choose.")).toBeInTheDocument();
    const search = screen.getByRole("search");
    const input = screen.getByRole("textbox", { name: "Search your local sources" });
    expect(input).toHaveAccessibleName("Search your local sources");
    expect(search).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Ready. LOOM does not upload your library.");
    expect(screen.getByRole("button", { name: "Add a folder" })).toBeEnabled();
  });

  it("focuses search from the global shortcut and announces the next action", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "reconcile_approved_roots") return {};
      if (command === "list_source_roots") return [];
      if (command === "library_stats") return emptyStats;
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    const input = screen.getByRole("textbox", { name: "Search your local sources" });
    expect(input).toHaveAttribute("aria-keyshortcuts", "Control+K Meta+K");
    fireEvent.keyDown(window, { key: "k", code: "KeyK", ctrlKey: true });

    expect(input).toHaveFocus();
    expect(screen.getByRole("status")).toHaveTextContent("Search focused. Type a query and press Enter.");
    expect(screen.getByRole("link", { name: "Skip to search and results" })).toHaveAttribute("href", "#main-content");
  });

  it("runs the primary keyboard retrieval flow and opens the bound original", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "reconcile_approved_roots") return {};
      if (command === "list_source_roots") return [];
      if (command === "library_stats") return readyStats;
      if (command === "search") return [hit];
      if (command === "open_artifact") return undefined;
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    const input = screen.getByRole("textbox", { name: "Search your local sources" });
    fireEvent.change(input, { target: { value: '"retry anomalies"' } });
    fireEvent.keyDown(input, { key: "Enter", code: "Enter", charCode: 13 });
    fireEvent.submit(screen.getByRole("search"));

    expect(await screen.findByRole("heading", { name: "Recovered sources" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Recovered sources" })).toHaveFocus();
    expect(screen.getByText("retry anomalies")).toBeInTheDocument();
    expect(screen.getByText("Why this matched")).toBeInTheDocument();
    expect(screen.getByText(hit.match_reason)).toBeInTheDocument();
    expect(screen.getByText("lines 2–2")).toBeInTheDocument();
    expect(screen.getByText("evidence attached")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Open original/ }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("open_artifact", {
        request: {
          artifact_id: hit.artifact_id,
          version_id: hit.version_id,
          content_hash: hit.content_hash,
        },
      });
    });
  });

  it("renders a PDF page anchor without collapsing it into a text-only location", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "reconcile_approved_roots") return {};
      if (command === "list_source_roots") return [];
      if (command === "library_stats") return readyStats;
      if (command === "search") return [pdfHit];
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    const input = screen.getByRole("textbox", { name: "Search your local sources" });
    fireEvent.change(input, { target: { value: "second page marker" } });
    fireEvent.submit(screen.getByRole("search"));

    expect(await screen.findByRole("heading", { name: "Recovered sources" })).toBeInTheDocument();
    expect(screen.getByText("PDF")).toBeInTheDocument();
    expect(screen.getByText("page 2 · lines 1–1")).toBeInTheDocument();
  });

  it("resolves a PDF evidence view and keeps the verified page visible", async () => {
    const evidenceView = {
      artifact_id: pdfHit.artifact_id,
      version_id: pdfHit.version_id,
      passage_id: pdfHit.passage_id,
      title: pdfHit.title,
      media_type: pdfHit.media_type,
      source_uri: pdfHit.source_uri,
      content_hash: pdfHit.content_hash,
      passage_text: "Second page marker",
      anchor: pdfHit.anchor,
      page_count: 3,
      extractor_id: "loom.pdf",
      extractor_version: "0.1.0",
      extraction_metadata: {},
    };
    invokeMock.mockImplementation(async (command) => {
      if (command === "reconcile_approved_roots") return {};
      if (command === "list_source_roots") return [];
      if (command === "library_stats") return readyStats;
      if (command === "search") return [pdfHit];
      if (command === "resolve_evidence") return evidenceView;
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    const input = screen.getByRole("textbox", { name: "Search your local sources" });
    fireEvent.change(input, { target: { value: "second page marker" } });
    fireEvent.submit(screen.getByRole("search"));
    fireEvent.click(await screen.findByRole("button", { name: "View evidence" }));

    expect(await screen.findByRole("heading", { name: "paper.pdf" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "paper.pdf", level: 2 })).toHaveFocus();
    expect(screen.getByText("PDF page 2 · loom.pdf 0.1.0")).toBeInTheDocument();
    expect(screen.getByText("Page 2 of 3")).toBeInTheDocument();
    expect(screen.getByText("Second page marker")).toBeInTheDocument();
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("resolve_evidence", {
        request: {
          artifact_id: pdfHit.artifact_id,
          version_id: pdfHit.version_id,
          passage_id: pdfHit.passage_id,
          content_hash: pdfHit.content_hash,
        },
      });
    });
  });

  it("renders an image region evidence map with rotation controls", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "reconcile_approved_roots") return {};
      if (command === "list_source_roots") return [];
      if (command === "library_stats") return readyStats;
      if (command === "search") return [imageHit];
      if (command === "resolve_evidence") return {
        ...imageHit,
        passage_text: "LOOM OCR marker 123",
        extractor_id: "loom.ocr",
        extractor_version: "0.1.0",
        extraction_metadata: { provider_id: "macos.vision" },
      };
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    const input = screen.getByRole("textbox", { name: "Search your local sources" });
    fireEvent.change(input, { target: { value: "LOOM OCR marker" } });
    fireEvent.submit(screen.getByRole("search"));
    fireEvent.click(await screen.findByRole("button", { name: "View evidence" }));

    expect(await screen.findByRole("heading", { name: "screenshot.png" })).toBeInTheDocument();
    expect(screen.getByText(/Image region 100,50 · 200×100px/)).toBeInTheDocument();
    const stage = screen.getByTestId("image-evidence-stage");
    expect(stage).toHaveAttribute("data-rotation", "0");
    expect(screen.getByRole("img", { name: "Image evidence canvas at 0 degrees" })).toHaveAttribute("tabindex", "0");
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(stage).toHaveAttribute("data-zoom", "1.25");
    fireEvent.click(screen.getByRole("button", { name: "Rotate evidence" }));
    expect(stage).toHaveAttribute("data-rotation", "90");
  });

  it("discloses stale evidence instead of showing an unverified passage", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "reconcile_approved_roots") return {};
      if (command === "list_source_roots") return [];
      if (command === "library_stats") return readyStats;
      if (command === "search") return [hit];
      if (command === "resolve_evidence") throw new Error("artifact is stale or unavailable: 11111111");
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    const input = screen.getByRole("textbox", { name: "Search your local sources" });
    fireEvent.change(input, { target: { value: "retry anomalies" } });
    fireEvent.submit(screen.getByRole("search"));
    fireEvent.click(await screen.findByRole("button", { name: "View evidence" }));

    expect(await screen.findByRole("heading", { name: "Source needs attention." })).toBeInTheDocument();
    expect(screen.getAllByRole("alert")[0]).toHaveTextContent("artifact is stale or unavailable");
    expect(screen.getByText("Re-index the selected folder, then retry this evidence result.")).toBeInTheDocument();
  });

  it("announces the loading state and disables duplicate searches", async () => {
    let resolveSearch: (results: typeof hit[]) => void = () => undefined;
    const pendingSearch = new Promise<typeof hit[]>((resolve) => {
      resolveSearch = resolve;
    });
    invokeMock.mockImplementation(async (command) => {
      if (command === "reconcile_approved_roots") return {};
      if (command === "list_source_roots") return [];
      if (command === "library_stats") return emptyStats;
      if (command === "search") return pendingSearch;
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    const input = screen.getByRole("textbox", { name: "Search your local sources" });
    fireEvent.change(input, { target: { value: "pending query" } });
    fireEvent.submit(screen.getByRole("search"));

    expect(screen.getByRole("status")).toHaveTextContent("Searching canonical local passages…");
    expect(screen.getByRole("button", { name: "Searching" })).toBeDisabled();
    resolveSearch([]);
    expect(await screen.findByRole("heading", { name: "No matching evidence" })).toBeInTheDocument();
  });

  it("shows partial indexing failures without hiding the usable library", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "reconcile_approved_roots") return {};
      if (command === "list_source_roots") return [];
      if (command === "library_stats") return readyStats;
      if (command === "index_selected_folder") {
        return {
          discovered: 2,
          indexed: 1,
          unchanged: 0,
          skipped: 1,
          bytes_read: 82,
          failures: [{ source: "/Users/test/notes/oversized.md", reason: "source exceeds limit" }],
        };
      }
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Add a folder" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("source exceeds limit");
    expect(screen.getByRole("status")).toHaveTextContent("Indexed 1; 0 unchanged; 1 unsupported; 1 failed.");
    expect(screen.getByText("1 original source ready to recover.")).toBeInTheDocument();
  });

  it("requests bounded cancellation and reports the resumable run", async () => {
    let resolveIndex: (report: unknown) => void = () => undefined;
    const pendingIndex = new Promise((resolve) => {
      resolveIndex = resolve;
    });
    invokeMock.mockImplementation(async (command) => {
      if (command === "reconcile_approved_roots") return {};
      if (command === "list_source_roots") return [];
      if (command === "library_stats") return readyStats;
      if (command === "index_selected_folder") return pendingIndex;
      if (command === "cancel_indexing") return true;
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Add a folder" }));
    fireEvent.click(screen.getByRole("button", { name: "Stop indexing" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("cancel_indexing");
    });
    resolveIndex({
      run_id: "12345678-1234-4123-8123-123456789012",
      discovered: 3,
      attempted: 1,
      indexed: 1,
      unchanged: 0,
      skipped: 0,
      failed: 0,
      cancelled: 2,
      bytes_read: 82,
      failures: [],
    });
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Cancelled run 12345678… after 1 unit; 2 remain resumable.",
    );
  });

  it("states when a valid search has no evidence instead of inventing an answer", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "reconcile_approved_roots") return {};
      if (command === "list_source_roots") return [];
      if (command === "library_stats") return emptyStats;
      if (command === "search") return [];
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    const input = screen.getByRole("textbox", { name: "Search your local sources" });
    fireEvent.change(input, { target: { value: "missing source" } });
    fireEvent.submit(screen.getByRole("search"));

    expect(await screen.findByRole("heading", { name: "No matching evidence" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "No matching evidence" })).toHaveFocus();
    expect(screen.getByRole("status")).toHaveTextContent("No evidence met this lexical query.");
  });

  it("shows bounded saved-scope states and revokes without widening access", async () => {
    const roots = [{
      locator: "/Users/test/notes",
      kind: "directory",
      enabled: true,
      read_only: true,
      status: "missing",
    }];
    invokeMock.mockImplementation(async (command) => {
      if (command === "reconcile_approved_roots") return { failures: [{ source: "/Users/test/notes", reason: "missing" }] };
      if (command === "list_source_roots") return roots;
      if (command === "library_stats") return emptyStats;
      if (command === "revoke_source_root") return { ...roots[0], enabled: false, status: "revoked" };
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);

    expect(await screen.findByText("missing — re-select")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Re-select" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Revoke /Users/test/notes" }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("revoke_source_root", { locator: "/Users/test/notes" });
    });
  });

  it("exposes a local OCR disable and purge control", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "reconcile_approved_roots") return {};
      if (command === "list_source_roots") return [];
      if (command === "library_stats") return emptyStats;
      if (command === "set_ocr_enabled") return { versions_deleted: 1, passages_deleted: 2 };
      if (command === "ocr_status") return { enabled: false, derived_versions: 0, derived_passages: 0 };
      throw new Error(`unexpected command: ${command}`);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Disable & purge" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("set_ocr_enabled", { enabled: false });
      expect(invokeMock).toHaveBeenCalledWith("ocr_status");
    });
    expect(await screen.findByText("disabled")).toBeInTheDocument();
  });
});

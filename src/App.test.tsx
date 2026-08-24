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
    expect(screen.getByText("retry anomalies")).toBeInTheDocument();
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
});

import { FormEvent, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import "./App.css";
import { compactPath } from "./evidence";

type LibraryStats = {
  source_roots: number;
  artifacts: number;
  versions: number;
  passages: number;
  indexed_bytes: number;
};

type IndexReport = {
  discovered: number;
  indexed: number;
  unchanged: number;
  skipped: number;
  bytes_read: number;
  failures: Array<{ source: string; reason: string }>;
};

type SearchHit = {
  rank: number;
  score: number;
  artifact_id: string;
  version_id: string;
  passage_id: string;
  title: string;
  media_type: string;
  source_uri: string;
  content_hash: string;
  excerpt: {
    segments: Array<{ text: string; highlighted: boolean }>;
  };
  anchor: {
    kind: "text";
    char_start: number;
    char_end: number;
    line_start: number;
    line_end: number;
  };
  match_reason: string;
};

const emptyStats: LibraryStats = {
  source_roots: 0,
  artifacts: 0,
  versions: 0,
  passages: 0,
  indexed_bytes: 0,
};

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function App() {
  const [stats, setStats] = useState<LibraryStats>(emptyStats);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [searched, setSearched] = useState(false);
  const [busy, setBusy] = useState<"index" | "search" | null>(null);
  const [notice, setNotice] = useState("Ready. LOOM does not upload your library.");
  const [error, setError] = useState<string | null>(null);

  const refreshStats = async () => {
    try {
      setStats(await invoke<LibraryStats>("library_stats"));
    } catch (caught) {
      setError(errorMessage(caught));
    }
  };

  useEffect(() => {
    let active = true;
    void invoke("reconcile_approved_roots")
      .then(() => invoke<LibraryStats>("library_stats"))
      .then((value) => {
        if (active) setStats(value);
      })
      .catch((caught: unknown) => {
        if (active) setError(errorMessage(caught));
      });
    return () => {
      active = false;
    };
  }, []);

  const addSource = async () => {
    setError(null);
    setBusy("index");
    setNotice("Choose a folder for LOOM to index…");
    try {
      const report = await invoke<IndexReport | null>("index_selected_folder");
      if (!report) {
        setNotice("Folder selection cancelled. Nothing was indexed.");
        return;
      }
      const failed = report.failures.length;
      setNotice(
        `Indexed ${report.indexed}; ${report.unchanged} unchanged; ${report.skipped} unsupported${failed ? `; ${failed} failed` : ""}.`,
      );
      if (failed) {
        setError(report.failures.map((item) => `${item.source}: ${item.reason}`).join("\n"));
      }
      await refreshStats();
    } catch (caught) {
      setError(errorMessage(caught));
      setNotice("Indexing stopped safely.");
    } finally {
      setBusy(null);
    }
  };

  const runSearch = async (event: FormEvent) => {
    event.preventDefault();
    if (!query.trim()) return;

    setError(null);
    setBusy("search");
    setSearched(true);
    setNotice("Searching canonical local passages…");
    try {
      const results = await invoke<SearchHit[]>("search", {
        request: { text: query, limit: 30 },
      });
      setHits(results);
      setNotice(
        results.length
          ? `Found ${results.length} source-backed result${results.length === 1 ? "" : "s"}.`
          : "No evidence met this lexical query.",
      );
    } catch (caught) {
      setHits([]);
      setError(errorMessage(caught));
      setNotice("Search stopped safely.");
    } finally {
      setBusy(null);
    }
  };

  const openArtifact = async (hit: SearchHit) => {
    setError(null);
    try {
      await invoke("open_artifact", {
        request: {
          artifact_id: hit.artifact_id,
          version_id: hit.version_id,
          content_hash: hit.content_hash,
        },
      });
    } catch (caught) {
      setError(errorMessage(caught));
    }
  };

  const headerMessage = useMemo(
    () =>
      stats.artifacts === 0
        ? "Start with a folder you intentionally choose."
        : `${stats.artifacts} original source${stats.artifacts === 1 ? "" : "s"} ready to recover.`,
    [stats.artifacts],
  );

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand-block">
          <div className="brand-mark" aria-hidden="true"><span /><span /><span /></div>
          <div><p className="brand">LOOM</p><p className="brand-subtitle">find the thread</p></div>
        </div>

        <nav aria-label="Library">
          <button className="nav-item active" type="button">
            <span className="nav-icon" aria-hidden="true">⌕</span>Search
          </button>
          <button className="nav-item" type="button" disabled title="Planned for a later release">
            <span className="nav-icon" aria-hidden="true">⟷</span>
            Relationships
            <span className="soon">later</span>
          </button>
        </nav>

        <section className="library-card" aria-labelledby="library-heading">
          <div className="section-label" id="library-heading">Local library</div>
          <dl className="stat-grid">
            <div><dt>Sources</dt><dd>{stats.artifacts}</dd></div>
            <div><dt>Passages</dt><dd>{stats.passages}</dd></div>
            <div><dt>Versions</dt><dd>{stats.versions}</dd></div>
            <div><dt>Indexed</dt><dd>{formatBytes(stats.indexed_bytes)}</dd></div>
          </dl>
          <button className="add-source" type="button" onClick={addSource} disabled={busy !== null}>
            <span aria-hidden="true">＋</span>
            {busy === "index" ? "Indexing…" : "Add a folder"}
          </button>
          <p className="scope-note">Text and Markdown only in this pre-alpha slice.</p>
        </section>

        <div className="privacy-note">
          <span className="privacy-dot" aria-hidden="true" />
          <div><strong>Local-only baseline</strong><span>No account · no telemetry · no network</span></div>
        </div>
      </aside>

      <main className="workspace">
        <header className="workspace-header">
          <div>
            <p className="eyebrow">Evidence-first personal retrieval</p>
            <h1>Recover the exact thing.</h1>
            <p className="header-copy">{headerMessage}</p>
          </div>
          <div className="status-pill"><span />pre-alpha</div>
        </header>

        <form className="search-form" onSubmit={runSearch} role="search">
          <span className="search-glyph" aria-hidden="true">⌕</span>
          <label className="sr-only" htmlFor="loom-query">Search your local sources</label>
          <input
            id="loom-query"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder='Try “the note about SQLite isolation”'
            autoComplete="off"
            spellCheck="false"
          />
          <kbd>↵</kbd>
          <button type="submit" disabled={busy !== null || !query.trim()}>
            {busy === "search" ? "Searching" : "Search"}
          </button>
        </form>

        <div className="notice" role="status" aria-live="polite">
          <span className={busy ? "pulse" : ""} />{notice}
        </div>
        {error && <pre className="error-panel" role="alert">{error}</pre>}

        {!searched && (
          <section className="empty-state">
            <div className="thread-visual" aria-hidden="true">
              <span className="spool one" /><span className="spool two" /><span className="spool three" />
            </div>
            <p className="empty-kicker">The source is the answer</p>
            <h2>Not another chat window.</h2>
            <p>
              LOOM returns the original file, the matched passage, its content hash, and the
              exact location that justified the result.
            </p>
            <div className="principles">
              <span>Exact source</span><span>Visible evidence</span><span>Local by default</span>
            </div>
          </section>
        )}

        {searched && hits.length === 0 && busy !== "search" && (
          <section className="no-results">
            <h2>No matching evidence</h2>
            <p>Try fewer terms or add a folder. LOOM will not invent an answer when it cannot find the source.</p>
          </section>
        )}

        {hits.length > 0 && (
          <section className="results" aria-label="Search results">
            <div className="results-heading">
              <h2>Recovered sources</h2>
              <span>{hits.length} matches · ranked with FTS5 BM25</span>
            </div>
            <ol>
              {hits.map((hit) => (
                <li key={hit.passage_id} className="result-card">
                  <div className="result-rank" aria-label={`Rank ${hit.rank}`}>
                    {String(hit.rank).padStart(2, "0")}
                  </div>
                  <div className="result-body">
                    <div className="result-title-row">
                      <div>
                        <span className="media-badge">{hit.media_type === "text/markdown" ? "MD" : "TXT"}</span>
                        <h3>{hit.title}</h3>
                      </div>
                      <button type="button" className="open-button" onClick={() => openArtifact(hit)}>
                        Open original <span aria-hidden="true">↗</span>
                      </button>
                    </div>
                    <p className="source-path" title={hit.source_uri}>{compactPath(hit.source_uri)}</p>
                    <blockquote>
                      {hit.excerpt.segments.map((part, index) =>
                        part.highlighted
                          ? <mark key={index}>{part.text}</mark>
                          : <span key={index}>{part.text}</span>,
                      )}
                    </blockquote>
                    <div className="evidence-row">
                      <span>lines {hit.anchor.line_start}–{hit.anchor.line_end}</span>
                      <span>score {hit.score.toFixed(4)}</span>
                      <span title={hit.content_hash}>{hit.content_hash.slice(0, 21)}…</span>
                      <span className="evidence-ok">evidence attached</span>
                    </div>
                  </div>
                </li>
              ))}
            </ol>
          </section>
        )}
      </main>
    </div>
  );
}

export default App;

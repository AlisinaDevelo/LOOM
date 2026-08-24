import { FormEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import "./App.css";
import { compactPath, projectImageRegion, type ImageRegionAnchor } from "./evidence";

type LibraryStats = {
  source_roots: number;
  artifacts: number;
  versions: number;
  passages: number;
  indexed_bytes: number;
};

type IndexReport = {
  run_id: string;
  discovered: number;
  attempted: number;
  indexed: number;
  unchanged: number;
  skipped: number;
  failed: number;
  cancelled: number;
  bytes_read: number;
  failures: Array<{ source: string; reason: string }>;
};

type ObservationReport = {
  failures?: Array<{ source: string; reason: string }>;
};

type SourceRootStatus = "available" | "missing" | "denied" | "wrong_type" | "unsafe" | "revoked" | "unavailable";

type SourceRootInfo = {
  locator: string;
  kind: string;
  enabled: boolean;
  read_only: boolean;
  status: SourceRootStatus;
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
    kind: "text" | "pdf_page" | "image_region";
    char_start: number;
    char_end: number;
    line_start: number;
    line_end: number;
    page?: number;
    x?: number;
    y?: number;
    width?: number;
    height?: number;
    image_width?: number;
    image_height?: number;
    orientation?: number;
    scale_milli?: number;
    confidence_milli?: number;
  };
  contributions: {
    lexical: number;
    semantic: number;
    metadata: number;
    reranker: number;
  };
  match_reason: string;
};

type EvidenceView = {
  artifact_id: string;
  version_id: string;
  passage_id: string;
  title: string;
  media_type: string;
  source_uri: string;
  content_hash: string;
  passage_text: string;
  anchor: SearchHit["anchor"];
  page_count?: number;
  extractor_id: string;
  extractor_version: string;
  extraction_metadata: Record<string, unknown>;
};

type EvidenceState = {
  hit: SearchHit;
  status: "loading" | "ready" | "error";
  view?: EvidenceView;
  error?: string;
};

type OcrStatus = {
  enabled: boolean;
  derived_versions: number;
  derived_passages: number;
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

function sourceRootStatusLabel(status: SourceRootStatus): string {
  return {
    available: "available",
    missing: "missing — re-select",
    denied: "denied — re-select",
    wrong_type: "moved — re-select",
    unsafe: "unsafe — re-select",
    revoked: "revoked",
    unavailable: "unavailable — re-select",
  }[status];
}

type EvidenceViewerProps = {
  state: EvidenceState;
  zoom: number;
  rotation: number;
  onZoomChange: (zoom: number) => void;
  onRotationChange: (rotation: number) => void;
  onClose: () => void;
  onOpenOriginal: (hit: SearchHit) => void;
};

function EvidenceViewer({
  state,
  zoom,
  rotation,
  onZoomChange,
  onRotationChange,
  onClose,
  onOpenOriginal,
}: EvidenceViewerProps) {
  const headingRef = useRef<HTMLHeadingElement>(null);
  const view = state.view;

  useEffect(() => {
    headingRef.current?.focus();
  }, [state.status, view?.artifact_id]);

  if (state.status === "loading") {
    return (
      <section className="evidence-viewer" aria-labelledby="evidence-viewer-title" role="region">
        <div className="evidence-viewer-heading">
          <div><p className="eyebrow">Verified evidence</p><h2 id="evidence-viewer-title" ref={headingRef} tabIndex={-1}>Checking the source…</h2></div>
          <button type="button" className="viewer-close" onClick={onClose}>Close</button>
        </div>
        <p className="evidence-loading" id="evidence-viewer-description">Re-checking the active version, content hash, and passage anchor.</p>
      </section>
    );
  }
  if (state.status === "error" || !view) {
    return (
      <section className="evidence-viewer evidence-viewer-error" aria-labelledby="evidence-viewer-title" role="region">
        <div className="evidence-viewer-heading">
          <div><p className="eyebrow">Evidence unavailable</p><h2 id="evidence-viewer-title" ref={headingRef} tabIndex={-1}>Source needs attention.</h2></div>
          <button type="button" className="viewer-close" onClick={onClose}>Close</button>
        </div>
        <p role="alert">{state.error ?? "The source could not be verified."}</p>
        <p className="evidence-loading" id="evidence-viewer-description">Re-index the selected folder, then retry this evidence result.</p>
      </section>
    );
  }

  const anchor = view.anchor;
  const isImage = anchor.kind === "image_region";
  const imageProjection = isImage
    ? projectImageRegion({
        x: anchor.x ?? 0,
        y: anchor.y ?? 0,
        width: anchor.width ?? 0,
        height: anchor.height ?? 0,
        image_width: anchor.image_width ?? 1,
        image_height: anchor.image_height ?? 1,
      } satisfies ImageRegionAnchor, zoom, rotation, window.devicePixelRatio || 1)
    : null;
  const location = anchor.kind === "pdf_page"
    ? `PDF page ${anchor.page ?? "?"}`
    : anchor.kind === "image_region"
      ? `Image region ${anchor.x ?? 0},${anchor.y ?? 0} · ${anchor.width ?? 0}×${anchor.height ?? 0}px`
      : `Text lines ${anchor.line_start}–${anchor.line_end}`;

  return (
    <section className="evidence-viewer" aria-labelledby="evidence-viewer-title" aria-describedby="evidence-viewer-description" role="region">
      <div className="evidence-viewer-heading">
        <div>
          <p className="eyebrow">Verified evidence</p>
          <h2 id="evidence-viewer-title" ref={headingRef} tabIndex={-1}>{view.title}</h2>
          <p className="evidence-location" id="evidence-viewer-description">{location} · {view.extractor_id} {view.extractor_version}</p>
        </div>
        <div className="viewer-actions">
          {isImage && <>
            <button type="button" className="viewer-control" onClick={() => onZoomChange(zoom - 0.25)} aria-label="Zoom out" aria-controls="image-evidence-stage">−</button>
            <span className="viewer-zoom" aria-live="polite" aria-atomic="true">{Math.round(zoom * 100)}%</span>
            <button type="button" className="viewer-control" onClick={() => onZoomChange(zoom + 0.25)} aria-label="Zoom in" aria-controls="image-evidence-stage">＋</button>
            <button type="button" className="viewer-control" onClick={() => onRotationChange(rotation + 90)} aria-label="Rotate evidence" aria-controls="image-evidence-stage">↻</button>
          </>}
          <button type="button" className="viewer-close" onClick={onClose}>Close</button>
        </div>
      </div>

      {isImage && imageProjection ? (
        <div
          className="evidence-stage image-stage"
          id="image-evidence-stage"
          data-testid="image-evidence-stage"
          data-rotation={imageProjection.rotation}
          data-zoom={imageProjection.scale}
          style={{ aspectRatio: `${imageProjection.canvasWidth} / ${imageProjection.canvasHeight}` }}
          role="group"
          aria-label="Verified image evidence"
        >
          <div
            className="image-evidence-map"
            style={{ aspectRatio: `${imageProjection.canvasWidth} / ${imageProjection.canvasHeight}` }}
            aria-label={`Image evidence canvas at ${imageProjection.rotation} degrees`}
            role="img"
            tabIndex={0}
          >
            <div
              className="image-evidence-region"
              style={{
                left: `${imageProjection.leftPercent}%`,
                top: `${imageProjection.topPercent}%`,
                width: `${imageProjection.widthPercent}%`,
                height: `${imageProjection.heightPercent}%`,
              }}
            />
            <span className="image-evidence-caption">OCR region · confidence {((anchor.confidence_milli ?? 0) / 10).toFixed(1)}%</span>
          </div>
        </div>
      ) : (
        <div className={`evidence-stage ${anchor.kind === "pdf_page" ? "pdf-stage" : "text-stage"}`}>
          {anchor.kind === "pdf_page" && <div className="pdf-page-label">Page {anchor.page ?? "?"} of {view.page_count ?? "?"}</div>}
          <blockquote className="verified-passage"><mark>{view.passage_text}</mark></blockquote>
          {anchor.kind === "pdf_page" && <div className="pdf-anchor-line" aria-hidden="true" />}
        </div>
      )}

      <div className="evidence-viewer-footer">
        <div>
          <span className="evidence-footer-label">Verified source</span>
          <code title={view.content_hash}>{view.content_hash.slice(0, 28)}…</code>
          <p title={view.source_uri}>{compactPath(view.source_uri)}</p>
        </div>
        <button type="button" className="open-button" onClick={() => onOpenOriginal(state.hit)}>
          Open original <span aria-hidden="true">↗</span>
        </button>
      </div>
    </section>
  );
}

function App() {
  const [stats, setStats] = useState<LibraryStats>(emptyStats);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [ocrStatus, setOcrStatus] = useState<OcrStatus>({
    enabled: true,
    derived_versions: 0,
    derived_passages: 0,
  });
  const [searched, setSearched] = useState(false);
  const [busy, setBusy] = useState<"index" | "search" | "scope" | "ocr" | "evidence" | null>(null);
  const [sourceRoots, setSourceRoots] = useState<SourceRootInfo[]>([]);
  const [evidenceState, setEvidenceState] = useState<EvidenceState | null>(null);
  const [evidenceZoom, setEvidenceZoom] = useState(1);
  const [evidenceRotation, setEvidenceRotation] = useState(0);
  const [notice, setNotice] = useState("Ready. LOOM does not upload your library.");
  const [error, setError] = useState<string | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const resultsHeadingRef = useRef<HTMLHeadingElement>(null);
  const noResultsHeadingRef = useRef<HTMLHeadingElement>(null);
  const focusResultsAfterSearchRef = useRef(false);

  const refreshStats = useCallback(async () => {
    try {
      setStats(await invoke<LibraryStats>("library_stats"));
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }, []);

  const refreshSourceRoots = useCallback(async () => {
    try {
      setSourceRoots(await invoke<SourceRootInfo[]>("list_source_roots"));
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }, []);

  const refreshLibrary = useCallback(async () => {
    await Promise.all([refreshStats(), refreshSourceRoots()]);
  }, [refreshSourceRoots, refreshStats]);

  useEffect(() => {
    let active = true;
    void invoke<ObservationReport>("reconcile_approved_roots")
      .then((report) => {
        const failures = report?.failures ?? [];
        if (active && failures.length) {
          setNotice(`${failures.length} saved folder${failures.length === 1 ? " needs" : "s need"} attention.`);
          setError(failures.map((item) => `${item.source}: ${item.reason}`).join("\n"));
        }
        return refreshLibrary();
      })
      .catch((caught: unknown) => {
        if (active) setError(errorMessage(caught));
      });
    return () => {
      active = false;
    };
  }, [refreshLibrary]);

  useEffect(() => {
    const focusSearch = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "k") return;
      const target = event.target as HTMLElement | null;
      const isOtherEditable = target && target !== searchInputRef.current
        && (target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName));
      if (isOtherEditable) return;
      event.preventDefault();
      searchInputRef.current?.focus();
      setNotice("Search focused. Type a query and press Enter.");
    };
    window.addEventListener("keydown", focusSearch);
    return () => window.removeEventListener("keydown", focusSearch);
  }, []);

  useEffect(() => {
    if (busy !== null || !searched || !focusResultsAfterSearchRef.current) return;
    focusResultsAfterSearchRef.current = false;
    (hits.length > 0 ? resultsHeadingRef : noResultsHeadingRef).current?.focus();
  }, [busy, hits.length, searched]);

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
      setNotice(report.cancelled
        ? `Cancelled run ${report.run_id.slice(0, 8)}… after ${report.attempted} unit${report.attempted === 1 ? "" : "s"}; ${report.cancelled} remain resumable.`
        : `Indexed ${report.indexed}; ${report.unchanged} unchanged; ${report.skipped} unsupported${failed ? `; ${failed} failed` : ""}.`);
      if (failed) {
        setError(report.failures.map((item) => `${item.source}: ${item.reason}`).join("\n"));
      }
      await refreshLibrary();
    } catch (caught) {
      setError(errorMessage(caught));
      setNotice("Indexing stopped safely.");
    } finally {
      setBusy(null);
    }
  };

  const cancelIndexing = async () => {
    setError(null);
    setNotice("Cancellation requested; LOOM will stop after the current bounded unit…");
    try {
      const accepted = await invoke<boolean>("cancel_indexing");
      if (!accepted) setNotice("No indexing run is active.");
    } catch (caught) {
      setError(errorMessage(caught));
      setNotice("Cancellation could not be requested.");
    }
  };

  const revokeSource = async (root: SourceRootInfo) => {
    setError(null);
    setBusy("scope");
    setNotice(`Revoking ${compactPath(root.locator)}…`);
    try {
      await invoke("revoke_source_root", { locator: root.locator });
      await refreshLibrary();
      setNotice("Folder revoked. Its indexed evidence is hidden until you explicitly re-select it.");
    } catch (caught) {
      setError(errorMessage(caught));
      setNotice("Folder revocation stopped safely.");
    } finally {
      setBusy(null);
    }
  };

  const toggleOcr = async () => {
    setError(null);
    setBusy("ocr");
    const next = !ocrStatus.enabled;
    setNotice(next ? "Enabling local image OCR…" : "Disabling OCR and purging derived records…");
    try {
      await invoke("set_ocr_enabled", { enabled: next });
      const status = await invoke<OcrStatus>("ocr_status");
      setOcrStatus(status);
      setNotice(next ? "Local image OCR is enabled." : "OCR is disabled; derived OCR records were purged.");
      await refreshStats();
    } catch (caught) {
      setError(errorMessage(caught));
      setNotice("OCR policy change stopped safely.");
    } finally {
      setBusy(null);
    }
  };

  const purgeOcr = async () => {
    setError(null);
    setBusy("ocr");
    setNotice("Purging derived OCR records…");
    try {
      await invoke("purge_ocr_records");
      const status = await invoke<OcrStatus>("ocr_status");
      setOcrStatus(status);
      setNotice("Derived OCR records purged; original images remain untouched.");
      await refreshStats();
    } catch (caught) {
      setError(errorMessage(caught));
      setNotice("OCR purge stopped safely.");
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
    focusResultsAfterSearchRef.current = true;
    setNotice("Searching canonical local passages…");
    try {
      const results = await invoke<SearchHit[]>("search", {
        request: { text: query, limit: 30 },
      });
      setHits(results);
      setEvidenceState(null);
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

  const resolveEvidence = async (hit: SearchHit) => {
    setError(null);
    setBusy("evidence");
    setEvidenceState({ hit, status: "loading" });
    setEvidenceZoom(1);
    setEvidenceRotation(0);
    try {
      const view = await invoke<EvidenceView>("resolve_evidence", {
        request: {
          artifact_id: hit.artifact_id,
          version_id: hit.version_id,
          passage_id: hit.passage_id,
          content_hash: hit.content_hash,
        },
      });
      setEvidenceState({ hit, status: "ready", view });
    } catch (caught) {
      const message = errorMessage(caught);
      setEvidenceState({ hit, status: "error", error: message });
      setError(message);
    } finally {
      setBusy(null);
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
      <a className="skip-link" href="#main-content">Skip to search and results</a>
      <aside className="sidebar">
        <div className="brand-block">
          <div className="brand-mark" aria-hidden="true"><span /><span /><span /></div>
          <div><p className="brand">LOOM</p><p className="brand-subtitle">find the thread</p></div>
        </div>

        <nav aria-label="Library">
          <button className="nav-item active" type="button" aria-current="page">
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
          <button
            className={busy === "index" ? "add-source cancel-source" : "add-source"}
            type="button"
            onClick={busy === "index" ? cancelIndexing : addSource}
            disabled={busy !== null && busy !== "index"}
          >
            <span aria-hidden="true">{busy === "index" ? "×" : "＋"}</span>
            {busy === "index" ? "Stop indexing" : "Add a folder"}
          </button>
          <p className="scope-note">Text, Markdown, bounded PDF text, and local image OCR are supported.</p>
          <section className="ocr-controls" aria-labelledby="ocr-heading">
            <div className="section-label" id="ocr-heading">Image OCR</div>
            <div className="ocr-control-row">
              <span>{ocrStatus.enabled ? "enabled" : "disabled"}</span>
              <button type="button" className="scope-action" onClick={toggleOcr} disabled={busy !== null}>
                {ocrStatus.enabled ? "Disable & purge" : "Enable"}
              </button>
            </div>
            <p className="scope-note">Runs on-device through macOS Vision. Disabling removes derived text and keeps original images.</p>
            <button type="button" className="scope-action" onClick={purgeOcr} disabled={busy !== null}>
              Purge derived OCR now
            </button>
          </section>
          <section className="scope-list" aria-labelledby="scope-heading">
            <div className="section-label" id="scope-heading">Saved scopes</div>
            {sourceRoots.length === 0 ? (
              <p className="scope-empty">No folders saved yet.</p>
            ) : (
              <ul>
                {sourceRoots.map((root) => (
                  <li key={root.locator} className={`scope-row scope-${root.status}`}>
                    <div className="scope-row-main">
                      <span className="scope-path" title={root.locator}>{compactPath(root.locator, 31)}</span>
                      <span className="scope-status">{sourceRootStatusLabel(root.status)}</span>
                    </div>
                    <div className="scope-actions">
                      {root.status !== "available" && root.status !== "revoked" && (
                        <button type="button" className="scope-action" onClick={addSource} disabled={busy !== null}>
                          Re-select
                        </button>
                      )}
                      {root.enabled && (
                        <button
                          type="button"
                          className="scope-action scope-revoke"
                          onClick={() => revokeSource(root)}
                          disabled={busy !== null}
                          aria-label={`Revoke ${root.locator}`}
                        >
                          Revoke
                        </button>
                      )}
                    </div>
                  </li>
                ))}
              </ul>
            )}
            <p className="scope-note">Saved paths persist locally and are read-only. Missing or denied folders never broaden access; re-select a folder explicitly to grant it again.</p>
          </section>
        </section>

        <div className="privacy-note">
          <span className="privacy-dot" aria-hidden="true" />
          <div><strong>Local-only baseline</strong><span>No account · no telemetry · no network</span></div>
        </div>
      </aside>

      <main className="workspace" id="main-content" tabIndex={-1} aria-labelledby="workspace-heading">
        <header className="workspace-header">
          <div>
            <p className="eyebrow">Evidence-first personal retrieval</p>
            <h1 id="workspace-heading">Recover the exact thing.</h1>
            <p className="header-copy">{headerMessage}</p>
          </div>
          <div className="status-pill"><span aria-hidden="true" />pre-alpha</div>
        </header>

        <form className="search-form" onSubmit={runSearch} role="search" aria-label="Search local sources">
          <span className="search-glyph" aria-hidden="true">⌕</span>
          <label className="sr-only" htmlFor="loom-query">Search your local sources</label>
          <input
            id="loom-query"
            ref={searchInputRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder='Try “the note about SQLite isolation”'
            autoComplete="off"
            spellCheck="false"
            aria-describedby="query-hint keyboard-hint"
            aria-keyshortcuts="Control+K Meta+K"
          />
          <span id="keyboard-hint" className="sr-only">Press Command or Control K to focus search. Press Enter to search.</span>
          <span className="keyboard-hints" aria-hidden="true">
            <kbd aria-hidden="true">⌘K</kbd><kbd aria-hidden="true">↵</kbd>
          </span>
          <button type="submit" disabled={busy !== null || !query.trim()}>
            {busy === "search" ? "Searching" : "Search"}
          </button>
        </form>
        <p className="query-hint" id="query-hint">
          Filters: <code>after:</code> <code>before:</code> <code>type:</code> <code>path:</code>{" "}
          <code>confidence:</code> · quoted text searches an exact phrase
        </p>

        <div className="notice" role="status" aria-live="polite" aria-atomic="true">
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
          <section className="no-results" aria-labelledby="no-results-heading">
            <h2 id="no-results-heading" ref={noResultsHeadingRef} tabIndex={-1}>No matching evidence</h2>
            <p>Try fewer terms or add a folder. LOOM will not invent an answer when it cannot find the source.</p>
          </section>
        )}

        {hits.length > 0 && (
          <section className="results" aria-labelledby="results-heading" aria-describedby="results-summary">
            <div className="results-heading">
              <h2 id="results-heading" ref={resultsHeadingRef} tabIndex={-1}>Recovered sources</h2>
              <span id="results-summary">{hits.length} matches · ranked with FTS5 BM25</span>
            </div>
            {evidenceState && (
              <EvidenceViewer
                state={evidenceState}
                zoom={evidenceZoom}
                rotation={evidenceRotation}
                onZoomChange={(next) => setEvidenceZoom(Math.max(0.5, Math.min(2, next)))}
                onRotationChange={(next) => setEvidenceRotation(((next % 360) + 360) % 360)}
                onClose={() => setEvidenceState(null)}
                onOpenOriginal={openArtifact}
              />
            )}
            <ol>
              {hits.map((hit) => (
                <li key={hit.passage_id} className="result-card">
                  <div className="result-rank" aria-label={`Rank ${hit.rank}`}>
                    {String(hit.rank).padStart(2, "0")}
                  </div>
                  <div className="result-body">
                    <div className="result-title-row">
                      <div>
                        <span className="media-badge">
                          {hit.media_type === "application/pdf" ? "PDF" : hit.media_type.startsWith("image/") ? "IMG" : hit.media_type === "text/markdown" ? "MD" : "TXT"}
                        </span>
                        <h3>{hit.title}</h3>
                      </div>
                      <div className="result-actions">
                        <button type="button" className="evidence-button" onClick={() => resolveEvidence(hit)} disabled={busy !== null}>
                          {busy === "evidence" && evidenceState?.hit.passage_id === hit.passage_id ? "Checking evidence" : "View evidence"}
                        </button>
                        <button type="button" className="open-button" onClick={() => openArtifact(hit)} disabled={busy !== null}>
                          Open original <span aria-hidden="true">↗</span>
                        </button>
                      </div>
                    </div>
                    <p className="source-path" title={hit.source_uri}>{compactPath(hit.source_uri)}</p>
                    <blockquote>
                      {hit.excerpt.segments.map((part, index) =>
                        part.highlighted
                          ? <mark key={index}>{part.text}</mark>
                          : <span key={index}>{part.text}</span>,
                      )}
                    </blockquote>
                    <p className="match-reason"><span>Why this matched</span>{hit.match_reason}</p>
                    <div className="evidence-row">
                      <span>
                        {hit.anchor.kind === "pdf_page" ? `page ${hit.anchor.page} · ` : ""}
                        {hit.anchor.kind === "image_region" ? `region ${hit.anchor.x},${hit.anchor.y} · ` : ""}
                        lines {hit.anchor.line_start}–{hit.anchor.line_end}
                      </span>
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

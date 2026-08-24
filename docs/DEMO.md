# Local MVP demo

The demo uses only the checked-in rights-clean fixtures and a temporary local database. It never
uploads source bytes or changes the repository.

```text
./scripts/demo-mvp.sh
```

The script prints a text recovery, a macOS Vision OCR-region recovery, local OCR status, library
counts, and the temporary corpus path. On the target Mac it is a compact smoke path for the current
MVP: selected-source ingest → exact lexical search → verified evidence → original handoff.

To see the desktop viewer, run the printed `npm run tauri -- dev` command, choose the printed
`sources` directory with **Add a folder**, search one of the demo phrases, and click **View evidence**.
The PDF/text panel shows the canonical passage and anchor; the image panel shows the OCR region,
confidence, and rotation/zoom controls. **Open original** remains a separately verified action.

The source authority is still the original path. The evidence panel does not claim to rasterize a
complete PDF or copy source image bytes into the webview. If a source changes after indexing, the
viewer reports a stale-source error and asks for re-indexing.

For any handoff screenshot, capture only the relevant result/evidence panel and crop away the rest
of the desktop before sharing. Do not include source paths, private documents, credentials, or the
full screen.

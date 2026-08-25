# LOOM explicit-save browser connector

This is the first implementation slice for roadmap `0301`. It captures only after a visible
toolbar click or `Ctrl+Shift+L`/`Command+Shift+L` command. It uses `activeTab` and `scripting` for
the current tab, has no host permissions, does not observe history/tabs/background events, and
sends only to the paired local native-messaging host `com.alisinadevelo.loom`.

The service worker captures the current DOM in the active tab's isolated execution world, then the
extension-owned sanitizer removes scripts, styles, forms, embedded resources, event attributes,
and every HTML attribute before the content is authenticated and sent. The canonical live URL,
final URL, title, selection, capture timestamp, redirect metadata, snapshot hash, parser version,
and complete/partial/failed/offline state stay separate. Oversized snapshots are UTF-8 bounded and
chunked under the native-messaging frame limit. No source content is written to logs, telemetry,
extension sync storage, or a network service.

The two manifests deliberately keep `host_permissions` empty. Chrome and Firefox manifests are
kept separate because their service-worker/background declarations differ. The repository now
also contains the checked Rust stdio receiver (`loom-native-host`), which validates every frame
and atomically spools only a complete, sanitized snapshot. Pairing UI/Keychain storage and the
signed host manifest are still packaging work; the receiver refuses to start without an explicit
secret and spool root, and the extension still refuses to inspect the active page without a
locally paired session (`pairing_required`).

## Contract tests

Run the offline contract suite from the repository root:

```text
node --test browser-extension/test/capture.test.js
python3 scripts/test-native-host.py --host target/debug/loom-native-host
```

The suite covers sanitization and remote-execution removal, authenticated payload construction,
same/cross-origin redirect semantics, oversized snapshots, offline pages, denied page access, and
the exact permission sets in both manifests. The native-host smoke test covers real framing,
malformed JSON, replay, hostile HTML, payload MAC failure, atomic spooling, and a fresh-counter
recovery. These are not a substitute for a consented Chrome and Firefox interactive permission
session; that device gate remains open until those browsers and a paired native host are exercised
with the release pairing flow. Any UI evidence must be cropped to the relevant browser/LOOM panel;
no full-screen screenshot is accepted.

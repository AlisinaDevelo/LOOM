/* global browser, btoa, document, location, URLSearchParams */
// Test-only extension page used by scripts/firefox-native-session.sh.
// Test-only page: drives the real extension capture builder and native-messaging port.
import {NATIVE_HOST, buildCaptureMessages, describeCaptureResponse} from "./src/capture.js";
const params = new URLSearchParams(location.search);
const secretBase64 = btoa(String.fromCharCode(...Array.from({length: 32}, (_, i) => i)));
const now = new Date();
const built = await buildCaptureMessages({
  page: {
    liveUrl: "https://example.test/firefox-session",
    finalUrl: "https://example.test/firefox-session",
    title: "Firefox native messaging session",
    selectedText: "synthetic firefox evidence",
    html: "<article><p>firefox-session-safe</p></article>",
    online: true,
    scope: "selection",
    redirects: [],
  },
  session: {
    id: "session-firefox", keyId: "pairing-key-firefox-session", secretBase64, counter: 0,
    issuedAt: now.toISOString(), expiresAt: new Date(now.getTime() + 300000).toISOString(),
  },
  intentToken: `intent-${params.get("run")}`,
  requestId: params.get("request"),
  capturedAt: now.toISOString(),
});
const done = (value) => { document.body.dataset.result = JSON.stringify(value); };
const port = browser.runtime.connectNative(NATIVE_HOST);
port.onMessage.addListener((response) => done({event: "message", lifecycle: describeCaptureResponse(response), type: response.type}));
port.onDisconnect.addListener((p) => { if (!document.body.dataset.result) done({event: "disconnect", error: p.error?.message ?? null}); });
for (const message of [built.request, ...built.payloads]) port.postMessage(message);

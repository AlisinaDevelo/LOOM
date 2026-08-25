import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  MAX_SNAPSHOT_BYTES,
  buildCaptureMessages,
  captureActiveTab,
  sanitizeHtml,
} from "../src/capture.js";

const SESSION_KEY = "test";
const SESSION = {
  id: "session-test-1",
  keyId: "pairing-key-test-only",
  secretBase64: SESSION_KEY,
  counter: 7,
  issuedAt: "2027-08-23T12:34:56Z",
  expiresAt: "2027-08-23T12:39:56Z",
};

function page(overrides = {}) {
  return {
    liveUrl: "https://example.test/start",
    finalUrl: "https://example.test/article",
    title: "Synthetic article",
    selectedText: "A selected passage.",
    html: "<article><h1>Synthetic article</h1><p>A selected passage.</p></article>",
    online: true,
    scope: "selection",
    redirects: ["https://example.test/start"],
    ...overrides,
  };
}

async function messages(overrides = {}) {
  return buildCaptureMessages({
    page: page(overrides.page),
    session: SESSION,
    intentToken: "intent-test-7",
    requestId: "11111111-1111-4111-8111-111111111111",
    capturedAt: "2027-08-23T12:34:56Z",
    ...overrides,
  });
}

test("sanitizes scripts, remote attributes, forms, and event handlers", () => {
  const sanitized = sanitizeHtml(
    '<article onclick="steal()"><script>fetch("https://bad.test")</script>' +
      '<img src="https://bad.test/x.png"><a href="javascript:alert(1)">Keep</a>' +
      "<form><input value=secret></form><p>Hello</p></article>",
  );

  assert.equal(sanitized, "<article> <a>Keep</a> <p>Hello</p></article>");
  assert.doesNotMatch(sanitized, /script|fetch|src=|href=|onclick|form|input|javascript:/i);
});

test("builds an authenticated complete request and chunked sanitized payload", async () => {
  const result = await messages();

  assert.equal(result.request.type, "capture.request");
  assert.equal(result.request.intent.kind, "user_gesture");
  assert.equal(result.request.source.live_url, "https://example.test/start");
  assert.equal(result.request.source.final_url, "https://example.test/article");
  assert.equal(result.request.snapshot.state, "complete");
  assert.match(result.request.auth.mac, /^[0-9a-f]{64}$/);
  assert.ok(result.payloads.length >= 1);
  assert.ok(result.payloads.every((payload) => payload.type === "capture.payload"));
  assert.deepEqual(Object.keys(result.payloads[0].session).sort(), ["counter", "id"]);
  assert.ok(result.payloads.every((payload) => /^[0-9a-f]+$/.test(payload.auth.mac)));
  assert.deepEqual(result.payloads.map((payload) => payload.sequence), [...result.payloads.keys()]);
  assert.ok(result.payloads.at(-1).final);
  assert.ok(result.payloads.every((payload, index) => payload.final === (index === result.payloads.length - 1)));
  assert.doesNotMatch(JSON.stringify(result), /<script|javascript:|src=|onclick/i);
});

test("marks cross-origin redirects partial and preserves both URLs", async () => {
  const result = await messages({
    page: page({
      finalUrl: "https://other.example.test/article",
      redirects: ["https://example.test/start", "https://other.example.test/article"],
    }),
  });

  assert.equal(result.request.source.live_url, "https://example.test/start");
  assert.equal(result.request.source.final_url, "https://other.example.test/article");
  assert.equal(result.request.snapshot.state, "partial");
  assert.equal(result.request.snapshot.failure_code, "cross_origin_redirect");
});

test("caps oversized snapshots as partial instead of sending unbounded bytes", async () => {
  const result = await messages({
    page: page({html: `<article>${"x".repeat(MAX_SNAPSHOT_BYTES + 4096)}</article>`}),
  });

  assert.equal(result.request.snapshot.state, "partial");
  assert.equal(result.request.snapshot.failure_code, "snapshot_truncated");
  assert.ok(result.request.snapshot.bytes <= MAX_SNAPSHOT_BYTES);
  assert.ok(result.payloads.length > 1);
});

test("offline pages retain source metadata but do not emit snapshot payloads", async () => {
  const result = await messages({page: page({online: false})});

  assert.equal(result.request.snapshot.state, "failed");
  assert.equal(result.request.snapshot.failure_code, "offline");
  assert.deepEqual(result.payloads, []);
  assert.equal(result.request.source.live_url, "https://example.test/start");
});

test("denied page access fails before native messaging and does not leak page data", async () => {
  let nativeCalls = 0;
  const api = {
    storage: {local: {get: async () => ({loomSession: SESSION})}},
    tabs: {query: async () => [{id: 42, url: "https://example.test/secret"}]},
    scripting: {executeScript: async () => { throw new Error("Cannot access contents of this page"); }},
    runtime: {connectNative: () => { nativeCalls += 1; throw new Error("must not connect"); }},
  };

  const result = await captureActiveTab(api);

  assert.deepEqual(result, {status: "failed", code: "capture_not_permitted"});
  assert.equal(nativeCalls, 0);
});

test("explicit save sends only authenticated request/payload frames and advances the counter", async () => {
  const posted = [];
  const stored = [];
  const api = {
    storage: {
      local: {
        get: async () => ({loomSession: SESSION}),
        set: async (value) => stored.push(value),
      },
    },
    tabs: {query: async () => [{id: 42, url: "https://example.test/start"}]},
    scripting: {executeScript: async () => [{result: page()}]},
    runtime: {connectNative: () => ({postMessage: (value) => posted.push(value)})},
  };

  const result = await captureActiveTab(api);

  assert.equal(result.status, "sent");
  assert.equal(posted[0].type, "capture.request");
  assert.ok(posted.slice(1).every((message) => message.type === "capture.payload"));
  assert.equal(stored.at(-1).loomSession.counter, SESSION.counter + 1);
  assert.ok(posted.every((message) => /^[0-9a-f]{64}$/.test(message.auth.mac)));
});

test("missing pairing is rejected before the active page is inspected", async () => {
  let executeCalls = 0;
  const api = {
    storage: {local: {get: async () => ({})}},
    tabs: {query: async () => [{id: 42, url: "https://example.test/secret"}]},
    scripting: {executeScript: async () => { executeCalls += 1; return []; }},
  };

  assert.deepEqual(await captureActiveTab(api), {status: "failed", code: "pairing_required"});
  assert.equal(executeCalls, 0);
});

test("background wiring exposes only explicit save actions", async () => {
  const source = await readFile(new URL("../src/background.js", import.meta.url), "utf8");

  assert.match(source, /onCommand/);
  assert.match(source, /onClicked/);
  assert.doesNotMatch(source, /history|tabs\.on|webRequest|setInterval|setTimeout/);
});

test("manifest permissions stay explicit and identical across browser targets", async () => {
  const [chrome, firefox] = await Promise.all([
    readFile(new URL("../manifest.chrome.json", import.meta.url), "utf8").then(JSON.parse),
    readFile(new URL("../manifest.firefox.json", import.meta.url), "utf8").then(JSON.parse),
  ]);
  const expected = ["activeTab", "commands", "nativeMessaging", "scripting", "storage"];

  assert.deepEqual(chrome.permissions.sort(), expected);
  assert.deepEqual(firefox.permissions.sort(), expected);
  assert.deepEqual(chrome.host_permissions, []);
  assert.deepEqual(firefox.host_permissions, []);
  assert.equal(chrome.commands["save-page"].suggested_key.default, "Ctrl+Shift+L");
  assert.equal(firefox.commands["save-page"].suggested_key.default, "Ctrl+Shift+L");
});

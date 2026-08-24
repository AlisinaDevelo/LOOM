const TEXT_ENCODER = new TextEncoder();
const TEXT_DECODER = new TextDecoder();

export const MAX_FRAME_BYTES = 256 * 1024;
export const MAX_SELECTED_TEXT_BYTES = 64 * 1024;
export const MAX_SNAPSHOT_BYTES = 8 * 1024 * 1024;
export const MAX_PAYLOAD_CHUNK_BYTES = 160 * 1024;
export const MAX_TITLE_BYTES = 512;
export const MAX_URL_BYTES = 16 * 1024;
export const MAX_REDIRECTS = 8;
export const PARSER_ID = "loom.web.sanitize";
export const PARSER_VERSION = "0.1.0";
export const NATIVE_HOST = "com.alisinadevelo.loom";
const REQUEST_ID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

const ALLOWED_TAGS = new Set([
  "a", "article", "blockquote", "br", "code", "dd", "div", "dl", "dt", "em", "figcaption",
  "figure", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hr", "li", "main", "mark",
  "ol", "p", "pre", "section", "small", "span", "strong", "table", "tbody", "td", "tfoot",
  "th", "thead", "tr", "ul",
]);

const VOID_TAGS = new Set(["br", "hr"]);
const DANGEROUS_TAGS = ["script", "style", "iframe", "object", "embed", "form", "base", "link", "meta", "template"];

function webCrypto() {
  if (!globalThis.crypto?.subtle) {
    throw new Error("Web Crypto is unavailable");
  }
  return globalThis.crypto;
}

function utf8Bytes(value) {
  return TEXT_ENCODER.encode(value);
}

function byteLength(value) {
  return utf8Bytes(value).byteLength;
}

function truncateUtf8(value, maximumBytes) {
  const bytes = utf8Bytes(value);
  if (bytes.byteLength <= maximumBytes) {
    return {value, truncated: false};
  }
  return {value: TEXT_DECODER.decode(bytes.slice(0, maximumBytes)), truncated: true};
}

function encodeBase64(bytes) {
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return btoa(binary);
}

function decodeBase64(value) {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function sortedValue(value) {
  if (Array.isArray(value)) {
    return value.map(sortedValue);
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.keys(value).sort().map((key) => [key, sortedValue(value[key])]));
  }
  return value;
}

export function canonicalJson(value) {
  return JSON.stringify(sortedValue(value));
}

async function digestHex(value) {
  const digest = await webCrypto().subtle.digest("SHA-256", utf8Bytes(value));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function hmacEnvelope(envelope, secretBase64) {
  const unsigned = structuredClone(envelope);
  delete unsigned.auth.mac;
  const key = await webCrypto().subtle.importKey(
    "raw",
    decodeBase64(secretBase64),
    {name: "HMAC", hash: "SHA-256"},
    false,
    ["sign"],
  );
  const signature = await webCrypto().subtle.sign("HMAC", key, utf8Bytes(canonicalJson(unsigned)));
  return [...new Uint8Array(signature)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function requireHttpUrl(value, field) {
  if (typeof value !== "string" || byteLength(value) > MAX_URL_BYTES) {
    throw new Error(`${field}_invalid`);
  }
  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    throw new Error(`${field}_invalid`);
  }
  if (!new Set(["http:", "https:"]).has(parsed.protocol) || !parsed.hostname) {
    throw new Error(`${field}_invalid`);
  }
  return parsed.href;
}

function normalizeText(value, maximumBytes) {
  const normalized = String(value ?? "").replace(/\s+/g, " ").trim();
  return truncateUtf8(normalized, maximumBytes).value;
}

function removeDangerousBlocks(html) {
  let output = String(html ?? "").replace(/<!--[\s\S]*?-->/g, " ");
  for (const tag of DANGEROUS_TAGS) {
    const pattern = new RegExp(`<${tag}(?:\\s[^>]*)?>[\\s\\S]*?(?:<\\/${tag}\\s*>|$)`, "gi");
    output = output.replace(pattern, " ");
  }
  return output;
}

/**
 * Conservative string sanitizer for the extension boundary.
 * All attributes are discarded, so links cannot retain remote targets and no
 * script, style, form, or embedded resource can enter the native payload.
 */
export function sanitizeHtml(html) {
  const withoutDangerousBlocks = removeDangerousBlocks(html);
  const withoutTags = withoutDangerousBlocks.replace(
    /<\/?([a-z][a-z0-9-]*)(?:\s[^>]*)?\/?>/gi,
    (whole, name) => {
      const tag = name.toLowerCase();
      if (!ALLOWED_TAGS.has(tag)) {
        return "";
      }
      if (whole.startsWith("</")) {
        return VOID_TAGS.has(tag) ? "" : `</${tag}>`;
      }
      return `<${tag}>`;
    },
  );
  return withoutTags.replace(/[\t\r\n ]+/g, " ").trim();
}

function sameOrigin(first, second) {
  try {
    return new URL(first).origin === new URL(second).origin;
  } catch {
    return false;
  }
}

function snapshotState(page, sanitizedHtml) {
  if (page.online === false) {
    return {state: "failed", failureCode: "offline", text: ""};
  }
  if (page.snapshotRequested === false) {
    return {state: "not_requested", failureCode: null, text: ""};
  }
  if (page.snapshotFailure) {
    return {state: "failed", failureCode: page.snapshotFailure, text: ""};
  }
  if (!sanitizedHtml) {
    return {state: "failed", failureCode: "empty_snapshot", text: ""};
  }
  const limited = truncateUtf8(sanitizedHtml, MAX_SNAPSHOT_BYTES);
  let state = limited.truncated ? "partial" : "complete";
  let failureCode = limited.truncated ? "snapshot_truncated" : null;
  if (!sameOrigin(page.liveUrl, page.finalUrl)) {
    state = "partial";
    failureCode = "cross_origin_redirect";
  }
  return {state, failureCode, text: limited.value};
}

async function payloadFrames({request, snapshotText, session}) {
  if (!snapshotText) {
    return [];
  }
  const bytes = utf8Bytes(snapshotText);
  const contentHash = request.snapshot.content_hash;
  const frames = [];
  let sequence = 0;
  for (let offset = 0; offset < bytes.length; offset += MAX_PAYLOAD_CHUNK_BYTES) {
    const chunk = bytes.slice(offset, offset + MAX_PAYLOAD_CHUNK_BYTES);
    const frame = {
      protocol: {major: 1, minor: 0},
      type: "capture.payload",
      request_id: request.request_id,
      session: request.session,
      sequence,
      final: offset + chunk.length >= bytes.length,
      content_type: "text/html",
      encoding: "base64",
      content_hash: contentHash,
      bytes: chunk.byteLength,
      body: encodeBase64(chunk),
      auth: {key_id: session.keyId, algorithm: "HMAC-SHA256", mac: ""},
    };
    frame.auth.mac = await hmacEnvelope(frame, session.secretBase64);
    if (byteLength(JSON.stringify(frame)) > MAX_FRAME_BYTES) {
      throw new Error("payload_frame_too_large");
    }
    frames.push(frame);
    sequence += 1;
  }
  return frames;
}

export async function buildCaptureMessages({page, session, intentToken, requestId, capturedAt}) {
  if (!session?.id || !session.keyId || !session.secretBase64) {
    throw new Error("pairing_required");
  }
  if (!intentToken || typeof intentToken !== "string") {
    throw new Error("capture_not_approved");
  }
  if (!REQUEST_ID_RE.test(requestId || "")) {
    throw new Error("request_id_invalid");
  }
  if (!Number.isInteger(session.counter) || session.counter < 0) {
    throw new Error("session_counter_invalid");
  }
  if (!capturedAt || Number.isNaN(Date.parse(capturedAt))) {
    throw new Error("capture_time_invalid");
  }
  const liveUrl = requireHttpUrl(page.liveUrl, "live_url");
  const finalUrl = requireHttpUrl(page.finalUrl || page.liveUrl, "final_url");
  const title = normalizeText(page.title, MAX_TITLE_BYTES);
  const selectedText = normalizeText(page.selectedText, MAX_SELECTED_TEXT_BYTES);
  const sanitizedHtml = sanitizeHtml(page.html || "");
  const snapshot = snapshotState({...page, liveUrl, finalUrl}, sanitizedHtml);
  const snapshotBytes = utf8Bytes(snapshot.text).byteLength;
  const snapshotHash = snapshot.text ? `sha256:${await digestHex(snapshot.text)}` : null;
  const counter = session.counter + 1;
  const request = {
    protocol: {major: 1, minor: 0},
    type: "capture.request",
    request_id: requestId,
    session: {
      id: session.id,
      counter,
      issued_at: session.issuedAt,
      expires_at: session.expiresAt,
    },
    intent: {kind: "user_gesture", token: intentToken, issued_at: capturedAt},
    source: {
      live_url: liveUrl,
      final_url: finalUrl,
      title,
      selected_text: selectedText,
      captured_at: capturedAt,
      redirects: (page.redirects || []).slice(0, MAX_REDIRECTS).map((url) => requireHttpUrl(url, "redirect")),
      scope: page.scope || "page",
    },
    snapshot: {
      state: snapshot.state,
      content_hash: snapshotHash,
      parser_id: snapshot.text ? PARSER_ID : null,
      parser_version: snapshot.text ? PARSER_VERSION : null,
      bytes: snapshotBytes,
      failure_code: snapshot.failureCode,
    },
    auth: {key_id: session.keyId, algorithm: "HMAC-SHA256", mac: ""},
  };
  if (!["selection", "page"].includes(request.source.scope)) {
    throw new Error("capture_scope_invalid");
  }
  request.auth.mac = await hmacEnvelope(request, session.secretBase64);
  if (byteLength(JSON.stringify(request)) > MAX_FRAME_BYTES) {
    throw new Error("request_frame_too_large");
  }
  return {
    request,
    payloads: await payloadFrames({request, snapshotText: snapshot.text, session}),
    nextCounter: counter,
  };
}

export function collectPageFromTab() {
  const maximumRawBytes = 8 * 1024 * 1024 + 1;
  const rawHtml = document.documentElement?.outerHTML || "";
  return {
    finalUrl: location.href,
    title: document.title || "",
    selectedText: globalThis.getSelection?.()?.toString() || "",
    html: rawHtml.slice(0, maximumRawBytes),
    truncated: rawHtml.length > maximumRawBytes,
    online: navigator.onLine !== false,
  };
}

export async function captureActiveTab(api) {
  const sessionResult = await api.storage.local.get("loomSession");
  const session = sessionResult?.loomSession;
  if (!session?.id || !session.keyId || !session.secretBase64) {
    return {status: "failed", code: "pairing_required"};
  }
  const tabs = await api.tabs.query({active: true, currentWindow: true});
  const tab = tabs[0];
  if (!tab?.id || !tab.url) {
    return {status: "failed", code: "no_active_tab"};
  }
  let page;
  try {
    const results = await api.scripting.executeScript({target: {tabId: tab.id}, func: collectPageFromTab});
    page = results?.[0]?.result;
  } catch {
    return {status: "failed", code: "capture_not_permitted"};
  }
  if (!page) {
    return {status: "failed", code: "capture_not_permitted"};
  }
  const requestId = crypto.randomUUID();
  const capturedAt = new Date().toISOString();
  let built;
  try {
    built = await buildCaptureMessages({
      page: {...page, liveUrl: tab.url},
      session,
      intentToken: crypto.randomUUID(),
      requestId,
      capturedAt,
    });
  } catch (error) {
    return {status: "failed", code: error instanceof Error ? error.message : "capture_invalid"};
  }
  let port;
  try {
    port = api.runtime.connectNative(NATIVE_HOST);
    for (const message of [built.request, ...built.payloads]) {
      port.postMessage(message);
    }
  } catch {
    return {status: "failed", code: "native_host_unavailable"};
  }
  await api.storage.local.set({loomSession: {...session, counter: built.nextCounter}});
  return {status: "sent", requestId, snapshotState: built.request.snapshot.state};
}

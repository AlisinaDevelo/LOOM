/* global console, process, setTimeout, WebSocket */
// Used by scripts/firefox-native-session.sh. Minimal WebDriver BiDi client: installs the staged extension temporarily and runs the harness.
const [,, port, extPath, uuid, runs] = process.argv;
const ws = new WebSocket(`ws://127.0.0.1:${port}/session`);
let id = 0; const pending = new Map();
const send = (method, params) => new Promise((resolve, reject) => {
  const n = ++id; pending.set(n, {resolve, reject}); ws.send(JSON.stringify({id: n, method, params}));
});
ws.onmessage = ({data}) => { const m = JSON.parse(data); const p = pending.get(m.id); if (!p) return;
  pending.delete(m.id);
  if (m.type === "error") p.reject(new Error(`${m.error}: ${m.message}`));
  else p.resolve(m.result);
};
await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
await send("session.new", {capabilities: {}});
const ext = await send("webExtension.install", {extensionData: {type: "path", path: extPath}});
console.log("installed", JSON.stringify(ext));
// A fresh profile may not have a top-level tab yet; open one when needed.
let {contexts} = await send("browsingContext.getTree", {});
const context = contexts[0]?.context ?? (await send("browsingContext.create", {type: "tab"})).context;
const results = [];
for (const [run, request] of JSON.parse(runs)) {
  await send("browsingContext.navigate", {context, url: `moz-extension://${uuid}/harness.html?run=${run}&request=${request}`, wait: "complete"});
  let value = null;
  for (let i = 0; i < 100 && !value; i++) {
    const r = await send("script.evaluate", {expression: "document.body?.dataset.result ?? null", target: {context}, awaitPromise: false});
    value = r.result?.value ?? null;
    if (!value) await new Promise((r) => setTimeout(r, 200));
  }
  results.push({run, result: value ? JSON.parse(value) : "timeout"});
}
console.log(JSON.stringify(results, null, 2));
await send("session.end", {}).catch(() => {});
ws.close();

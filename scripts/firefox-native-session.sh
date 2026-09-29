#!/usr/bin/env bash
# Real Firefox native-messaging session for the explicit-save extension.
#
# Loads the extension as a temporary add-on in a throwaway headless Firefox profile over WebDriver
# BiDi, registers the built loom-native-host for the duration of the run, and checks three
# outcomes against the real browser: a fresh save is accepted, a replay on a new connection is
# rejected without changing the stored record, and an unpaired extension is refused. The page
# uses the real capture builder and native-messaging port; it does not exercise the toolbar
# gesture or activeTab grant, which still need a person.
set -Eeuo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
HOST="$ROOT/target/debug/loom-native-host"
FIREFOX=${FIREFOX:-/Applications/Firefox.app/Contents/MacOS/firefox}
PORT=${LOOM_BIDI_PORT:-9333}
EXTENSION_ID="loom-explicit-save@alisinadevelo.local"
MANIFEST_DIR="$HOME/Library/Application Support/Mozilla/NativeMessagingHosts"
MANIFEST="$MANIFEST_DIR/com.alisinadevelo.loom.json"

[[ -x "$HOST" ]] || { echo "build the host first: cargo build --locked -p loom --bin loom-native-host" >&2; exit 2; }
[[ -x "$FIREFOX" ]] || { echo "Firefox not found at $FIREFOX" >&2; exit 2; }
[[ -e "$MANIFEST" ]] && { echo "refusing to replace an existing $MANIFEST" >&2; exit 2; }

WORK=$(mktemp -d /tmp/loom-firefox-session.XXXXXX)
FIREFOX_PID=""
cleanup() {
  if [[ -n "$FIREFOX_PID" ]]; then
    kill "$FIREFOX_PID" 2>/dev/null || true
    wait "$FIREFOX_PID" 2>/dev/null || true
  fi
  grep -q "temporary test registration" "$MANIFEST" 2>/dev/null && rm -f "$MANIFEST"
  rm -rf "$WORK"
}
trap cleanup EXIT

mkdir -p "$WORK/ext" "$WORK/profile" "$WORK/spool" "$MANIFEST_DIR"
cp -R "$ROOT/browser-extension/src" "$WORK/ext/src"
cp "$ROOT/browser-extension/manifest.firefox.json" "$WORK/ext/manifest.json"
cp "$ROOT/scripts/firefox-session/harness.html" "$ROOT/scripts/firefox-session/harness.js" "$WORK/ext/"
UUID=$(python3 -c 'import uuid; print(uuid.uuid4())')
python3 - "$WORK/profile/user.js" "$EXTENSION_ID" "$UUID" <<'PY'
import json, sys
path, extension, uuid = sys.argv[1:]
prefs = {
    "extensions.webextensions.uuids": json.dumps({extension: uuid}),
    "browser.shell.checkDefaultBrowser": False,
    "datareporting.policy.dataSubmissionEnabled": False,
    "toolkit.telemetry.enabled": False,
}
with open(path, "w") as handle:
    for key, value in prefs.items():
        handle.write(f"user_pref({json.dumps(key)}, {json.dumps(value)});\n")
PY

echo "firefox:$EXTENSION_ID" > "$WORK/allowed_callers"
SECRET=$(python3 -c 'print(bytes(range(32)).hex())')
cat > "$WORK/host.sh" <<SH
#!/bin/bash
export LOOM_NATIVE_HOST_KEY_ID="pairing-key-firefox-session"
export LOOM_NATIVE_HOST_SECRET_HEX="$SECRET"
export LOOM_NATIVE_HOST_SPOOL="$WORK/spool"
export LOOM_NATIVE_HOST_ALLOWED_CALLERS="\$(cat "$WORK/allowed_callers")"
exec "$HOST" "\$@" 2>>"$WORK/host-stderr.log"
SH
chmod 700 "$WORK/host.sh"
python3 - "$MANIFEST" "$WORK/host.sh" "$EXTENSION_ID" <<'PY'
import json, sys
manifest, path, extension = sys.argv[1:]
json.dump({"name": "com.alisinadevelo.loom", "description": "LOOM native host (temporary test registration)",
           "path": path, "type": "stdio", "allowed_extensions": [extension]}, open(manifest, "w"), indent=2)
PY

# Runs one scenario in a fresh Firefox. Called directly (not in a subshell) so the EXIT trap
# always knows which Firefox to stop; the result is written to $WORK/<name>.json.
session() {
  local name=$1 request=$2
  if [[ -n "$FIREFOX_PID" ]]; then
    kill "$FIREFOX_PID" 2>/dev/null || true
    wait "$FIREFOX_PID" 2>/dev/null || true
  fi
  "$FIREFOX" --headless --no-remote --profile "$WORK/profile" --remote-debugging-port "$PORT" > "$WORK/firefox.log" 2>&1 &
  FIREFOX_PID=$!
  for _ in $(seq 1 60); do grep -q "WebDriver BiDi listening" "$WORK/firefox.log" && break; sleep 0.5; done
  node "$ROOT/scripts/firefox-session/drive.mjs" "$PORT" "$WORK/ext" "$UUID" "[[\"$name\",\"$request\"]]" \
    | python3 -c 'import json,sys; print(json.dumps(json.loads(sys.stdin.read().split("\n",1)[1])[0]["result"]))' \
    > "$WORK/$name.json"
}

REQUEST=77777777-7777-4777-8777-777777777777
session paired "$REQUEST"
before=$(shasum -a 256 "$WORK/spool/$REQUEST.json" | cut -d' ' -f1)
session replay "$REQUEST"
after=$(shasum -a 256 "$WORK/spool/$REQUEST.json" | cut -d' ' -f1)
echo "firefox:another-extension@example.test" > "$WORK/allowed_callers"
session unpaired 88888888-8888-4888-8888-888888888888
paired=$(cat "$WORK/paired.json")
replay=$(cat "$WORK/replay.json")
unpaired=$(cat "$WORK/unpaired.json")

echo "paired:   $paired"
echo "replay:   $replay"
echo "unpaired: $unpaired"
grep -q '"capture.accepted"' <<<"$paired" || { echo "FAIL: paired save was not accepted" >&2; exit 1; }
grep -q '"replay_rejected"' <<<"$replay" || { echo "FAIL: replay was not rejected" >&2; exit 1; }
[[ "$before" == "$after" ]] || { echo "FAIL: replay changed the stored record" >&2; exit 1; }
grep -q '"disconnect"' <<<"$unpaired" || { echo "FAIL: unpaired extension was not refused" >&2; exit 1; }
if grep -q "$REQUEST" "$WORK/spool/.loom-replay-ledger.json"; then
  echo "FAIL: replay ledger stores a raw request ID" >&2; exit 1
fi
echo "firefox native session: PASS (Firefox $("$FIREFOX" --version | awk '{print $3}'); accepted, replay rejected, unpaired refused)"

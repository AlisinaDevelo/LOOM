import {captureActiveTab} from "./capture.js";

const api = globalThis.browser ?? globalThis.chrome;

async function saveCurrentPage() {
  if (!api) {
    return {status: "failed", code: "browser_api_unavailable"};
  }
  const result = await captureActiveTab(api);
  // Store status only; source content never enters extension logs or telemetry.
  await api.storage.local.set({loomLastCapture: {at: new Date().toISOString(), ...result}});
  return result;
}

if (api?.commands?.onCommand) {
  api.commands.onCommand.addListener((command) => {
    if (command === "save-page") {
      void saveCurrentPage();
    }
  });
}

if (api?.action?.onClicked) {
  api.action.onClicked.addListener(() => {
    void saveCurrentPage();
  });
}

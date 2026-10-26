import { endpointUrl, validToken } from "./protocol.js";

export const errors = {
  invalid_endpoint: "Copy the local receiver address from Traeva → Source Memory → Connect Chrome extension. It must be http://127.0.0.1:PORT.",
  invalid_token: "Copy the complete pairing token from the same Traeva instance.",
  pairing_rejected: "The pairing information does not match this Traeva library. Copy its current address and token, then save settings. Waiting records have not been acknowledged as saved.",
  receiver_unavailable: "Cannot reach Traeva. Open your paired app, turn on Chrome collection in Source Memory, and compare its address with the saved address here. This connection cannot tell whether the app is closed, collection is off, or the address has changed.",
  receiver_rejected: "Traeva did not acknowledge this delivery. Waiting records will retry; check Source Memory for an app error.",
  record_rejected: "Traeva rejected this record or its size. Automatic retries stopped. Check Source Memory, then try the manual import after resolving the problem.",
  invalid_response: "This address did not return a Traeva response. Copy the address and token from your paired app.",
  collection_disabled: "Chrome collection is off in Traeva. Enable it in Source Memory. Downloads outside the enabled interval need an explicit manual import.",
  download_unavailable: "Chrome could not read this download record. It remains waiting and will retry.",
  delivery_failed: "The operation did not finish. Check the saved state here and in Traeva before retrying.",
  pending_clear_confirmation: "New waiting deliveries arrived. Review the warning and explicitly allow clearing them before saving this settings change.",
  no_folders: "No currently available authorized folders. In Traeva → Settings, authorize your download folder or restore access to it, then check again. Skipped downloads need a manual import.",
  outside_folders: "Not sent: the download is outside the currently available authorized folders. Authorize its folder in Traeva → Settings before a manual import.",
  outside_interval: "Not sent: this download started outside the enabled collection interval. Use the manual import if you want to include it.",
  receiver_ignored: "Traeva declined to collect this record under its current consent or folder rules. It was not acknowledged as saved. Check the app before a manual import.",
  download_erased: "Chrome no longer has this download record. Automatic retries stopped. Its source cannot be recovered from this record; any earlier save remains in Traeva.",
  waiting_filename: "Waiting for Chrome to provide a file location.",
  waiting_completion: "A progress record is saved in Traeva. Waiting for Chrome to report completion or interruption.",
};

export function connectionText(value) {
  if (!value.config.enabled) return "Collection is off in this extension. Enable Collect new downloads and save settings to begin.";
  const connection = value.status.connection;
  if (!connection) return "Settings are saved. Connection has not been checked yet.";
  if (connection.kind === "connected") return "Connected to Traeva. Download into a folder listed below, then look for its source in the app. A saved source still needs your confirmation to associate it with a file.";
  return errors[connection.kind] ?? errors.delivery_failed;
}

export function receiptText(entry) {
  if (entry.outcome !== "saved") return errors[entry.reason] ?? "No save was acknowledged. Review this download in Traeva.";
  if (entry.downloadState === "in_progress") return errors.waiting_completion;
  if (entry.downloadState === "interrupted") return "Source record saved in Traeva; Chrome reported an interrupted download. This does not mean the file finished downloading.";
  return "Source record saved in Traeva. Review its suggested file match in Source Memory.";
}

export function clearsPending(config, input) {
  if (!input.enabled) return true;
  try {
    if (!validToken(input.token)) return false;
    return endpointUrl(input.endpoint) !== config.endpoint || input.token !== config.token;
  } catch { return false; }
}

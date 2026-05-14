#!/usr/bin/env bun
import { spawnSync } from "node:child_process";

const appId = process.argv[2] ?? "com.local.sterm";
function run(command, args, { allowFailure = false, capture = false } = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit",
  });

  if (!allowFailure && (result.error || result.status !== 0)) {
    if (result.error) console.error(result.error.message);
    process.exit(result.status ?? 1);
  }

  return result.stdout ?? "";
}

const pid = run("adb", ["shell", "pidof", appId], { capture: true })
  .replace(/\r/g, "")
  .trim()
  .split(/\s+/)[0];

if (!pid) {
  console.error(`Could not find a running Android process for ${appId}. Start the app first.`);
  process.exit(1);
}

run("adb", ["forward", "--remove", "tcp:9222"], { allowFailure: true });
run("adb", ["forward", "tcp:9222", `localabstract:webview_devtools_remote_${pid}`]);

console.log("Forwarded Android WebView DevTools to http://127.0.0.1:9222");

try {
  const response = await fetch("http://127.0.0.1:9222/json");
  console.log(await response.text());
} catch (error) {
  console.error(`Forwarding succeeded, but DevTools JSON could not be read: ${error.message}`);
}

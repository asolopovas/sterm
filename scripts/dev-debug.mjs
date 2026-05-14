#!/usr/bin/env bun
import { spawn } from "node:child_process";

const bun = process.platform === "win32" ? "bun.exe" : "bun";
const child = spawn(bun, ["tauri", "dev"], {
  stdio: "inherit",
  env: {
    ...process.env,
    RUST_BACKTRACE: process.env.RUST_BACKTRACE ?? "1",
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:
      process.env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS ?? "--remote-debugging-port=9223",
  },
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 0);
});

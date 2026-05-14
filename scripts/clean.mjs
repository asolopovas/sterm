#!/usr/bin/env bun
import { rmSync } from "node:fs";
import { spawnSync } from "node:child_process";

rmSync("dist", { recursive: true, force: true });

const result = spawnSync("cargo", ["clean", "--manifest-path", "src-tauri/Cargo.toml"], {
  stdio: "inherit",
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 0);

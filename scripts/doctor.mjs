#!/usr/bin/env bun
import { spawnSync } from "node:child_process";

const includeAndroid = process.argv.includes("android");

function run(command, args) {
  const result = spawnSync(command, args, { stdio: "inherit" });

  if (result.error) {
    console.error(`Missing required command: ${command}`);
    console.error(result.error.message);
    process.exit(1);
  }

  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

if (process.versions.bun) {
  console.log(`bun ${process.versions.bun}`);
} else {
  run("bun", ["--version"]);
}
run("cargo", ["--version"]);
run("rustc", ["--version"]);
run("bun", ["tauri", "--version"]);

if (includeAndroid) {
  run("adb", ["version"]);
}

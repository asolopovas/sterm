#!/usr/bin/env bun
import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, statSync } from "node:fs";
import { createWriteStream } from "node:fs";
import { resolve } from "node:path";
import { once } from "node:events";
import { finished } from "node:stream/promises";

const appId = process.argv[2] ?? "com.local.sterm";
const outputDir = process.argv[3] ?? "tmp";
const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
const outputPath = resolve(outputDir, `android-snapshot-${timestamp}.png`);

function run(command, args) {
  const result = spawnSync(command, args, { stdio: "inherit" });

  if (result.error) {
    console.error(result.error.message);
    process.exit(1);
  }

  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

mkdirSync(outputDir, { recursive: true });

run("adb", ["shell", "monkey", "-p", appId, "-c", "android.intent.category.LAUNCHER", "1"]);
await new Promise((resolveDelay) => setTimeout(resolveDelay, 1000));

const adb = spawn("adb", ["exec-out", "screencap", "-p"], {
  stdio: ["ignore", "pipe", "inherit"],
});

adb.on("error", (error) => {
  console.error(error.message);
  process.exit(1);
});

const output = createWriteStream(outputPath);
adb.stdout.pipe(output);

const [status] = await once(adb, "close");
await finished(output);

if (status !== 0) {
  process.exit(status ?? 1);
}

const bytes = statSync(outputPath).size;
if (bytes === 0) {
  console.error(`Screenshot was empty: ${outputPath}`);
  process.exit(1);
}

console.log(`Saved Android snapshot: ${outputPath}`);

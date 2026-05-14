---
name: tauri-debugging
description: Inspect and debug running Tauri 2 apps on desktop, Android, and iOS — WebView inspector, Chrome DevTools Protocol (CDP) live JS eval, Safari Web Inspector, logcat triage, Rust panic capture, IPC/capability errors, and the HMR dev loop. Use whenever you need to diagnose a Tauri app's frontend, backend, or native bridge in real time.
license: MIT
---

# Tauri 2 debugging

Inspect a **running** Tauri 2 app. Every technique here works against a live process — you read state from the real WebView and the real Rust binary, not from source guesses.

Authoritative sources: [v2.tauri.app/develop/debug](https://v2.tauri.app/develop/debug/), [v2.tauri.app/develop](https://v2.tauri.app/develop/), [v2.tauri.app/plugin/logging](https://v2.tauri.app/plugin/logging/), [crabnebula-dev/devtools](https://github.com/crabnebula-dev/devtools).

## When to use

- A Tauri app is running and something is wrong (crash, blank screen, command failing, layout off, IPC denied, WS disconnect).
- You need to verify DOM/CSS/JS state on a real device without screenshots.
- You need to correlate a frontend symptom with a Rust-side cause.
- You're standing up the HMR dev loop on Android / iOS and the WebView won't connect.

## Core principle: prefer CDP/eval over screenshots

For anything measurable (sizes, classes, computed styles, element counts, fetch state, JS errors) — **use a CDP eval, not PNGs**. Screenshots are for visual judgment only (font rendering, animation feel, composition). CDP gives exact values; screenshots give vibes.

---

## Tauri 2 fundamentals

### Build modes

| Mode               | Command                                  | DevTools | Optimizations | Bundle path                        |
| ------------------ | ---------------------------------------- | -------- | ------------- | ---------------------------------- |
| Dev (HMR)          | `tauri dev`                              | on       | none          | runs from `target/debug`           |
| Debug build        | `tauri build --debug`                    | on       | none          | `src-tauri/target/debug/bundle/`   |
| Release            | `tauri build`                            | **off**  | full          | `src-tauri/target/release/bundle/` |
| Release + DevTools | `tauri build` + `devtools` Cargo feature | on       | full          | as above                           |

Enable DevTools in production by adding the `devtools` Cargo feature in `src-tauri/Cargo.toml`. Don't ship that to end users.

```toml
[dependencies]
tauri = { version = "2", features = ["devtools"] }
```

### Detecting build mode in code

```rust
#[cfg(dev)]                    // tauri dev only
#[cfg(debug_assertions)]       // tauri dev OR tauri build --debug
let is_dev: bool = tauri::is_dev();   // runtime check
```

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() { /* ... */ }
```

`#[cfg(mobile)]` / `#[cfg(desktop)]` gate platform-specific code. Mobile-only plugins (e.g. `tauri-plugin-nfc`) must be registered inside a `#[cfg(mobile)]` block.

### Programmatic DevTools control (debug builds)

```rust
use tauri::Manager;
tauri::Builder::default().setup(|app| {
    #[cfg(debug_assertions)]
    {
        let w = app.get_webview_window("main").unwrap();
        w.open_devtools();
        w.close_devtools();
    }
    Ok(())
});
```

Methods: `WebviewWindow::open_devtools()` / `close_devtools()`. **Not supported on Android** — use `chrome://inspect` instead.

---

## Desktop

### WebView inspector

- Right-click → **Inspect Element**, or `Ctrl+Shift+I` (Win/Linux), `Cmd+Option+I` (macOS).
- Backend per OS: **Edge DevTools** on Windows, **WebKit Web Inspector** on macOS, **WebKitGTK Web Inspector** on Linux.

### Rust core debugging

Run with backtraces:

```sh
RUST_BACKTRACE=1 tauri dev          # short trace
RUST_BACKTRACE=full tauri dev       # full trace with line numbers
```

PowerShell:

```powershell
$env:RUST_BACKTRACE = "1"; tauri dev
```

`RUST_LOG=tauri=debug,wry=debug,tao=debug` for IPC, window, and webview internals.

### Console capture on Windows release builds

By default a Tauri release `.exe` has no attached console, so `println!`/`eprintln!`/panics vanish. Options:

- Run from a terminal — `cmd` / PowerShell will see stdout/stderr.
- Add `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` only in release; debug builds get a console window.
- Use `tauri-plugin-log` with `Stdout` + `LogDir` targets so logs persist regardless of console attachment (see Logging section).

### LLDB / VS Code

`.vscode/launch.json` (uses `vscode-lldb`):

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "lldb",
      "request": "launch",
      "name": "Tauri Dev Debug",
      "cargo": {
        "args": ["build", "--manifest-path=./src-tauri/Cargo.toml", "--no-default-features"]
      },
      "preLaunchTask": "ui:dev"
    }
  ]
}
```

`preLaunchTask` should start the frontend dev server (your `beforeDevCommand` equivalent) as a background task in `.vscode/tasks.json`. On Windows, `cppvsdbg` (Visual Studio debugger via the C/C++ extension) is faster than LLDB and handles Rust enums better.

### Common desktop failures

| Symptom                                                        | First check                                                                  |
| -------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| Blank window                                                   | DevTools console → CSP/script errors. Then `RUST_LOG=wry=debug` on load.     |
| `invoke` returns "command not found"                           | Handler missing from `invoke_handler![…]` in `lib.rs`.                       |
| `... not allowed. Permissions associated with this command: …` | Missing capability. Add permission to `src-tauri/capabilities/*.json`.       |
| Serde error crossing JS↔Rust                                   | TS shape doesn't match Rust struct. Check `serde(rename_all = "camelCase")`. |
| App exits silently on launch                                   | `RUST_BACKTRACE=full` + run from console; usually a panic in `setup`.        |
| DevTools toggle disabled in release                            | Add `devtools` Cargo feature, or use `tauri build --debug`.                  |

---

## Android

Android Tauri runs the frontend in the system WebView (Chromium-based) inside an Android app. Two debug surfaces: the WebView (CDP / Chrome DevTools) and the Android process (logcat). **Wry's programmatic devtools API is not supported on Android** — use Chrome DevTools.

### Prerequisites

- `adb` on PATH; device in **Developer Mode** + **USB debugging**.
  - Developer Mode: Settings → About → tap **Build Number** 7 times, then Settings → Developer Options → USB debugging.
- Debug APK installed (release builds disable WebView debugging unless `WebView.setWebContentsDebuggingEnabled(true)` is forced; Tauri debug builds enable it by default).
- `adb devices` shows the device. With multiple devices, prefix every command with `-s <serial>`.
- **Emulators have inspector enabled by default**; physical devices need USB debugging on.

### Path 1 — `chrome://inspect` (official, GUI)

1. Run `tauri android dev` (or your project's equivalent recipe).
2. Open `chrome://inspect/#devices` in desktop Chrome.
3. The device + WebView appear in the remote target list.
4. Click **inspect** → full DevTools (DOM, network, console, sources/breakpoints, profiler).

### Path 2 — manual `adb forward` (scriptable, agent-friendly)

Locate the WebView's debug socket and forward it:

```sh
adb shell cat /proc/net/unix | grep webview_devtools_remote
# webview_devtools_remote_<pid>
adb forward tcp:9222 localabstract:webview_devtools_remote_<pid>
```

After forwarding, `chrome://inspect` works **and** programmatic CDP clients work against `http://localhost:9222`:

```sh
curl http://localhost:9222/json     # lists targets, includes webSocketDebuggerUrl
```

Many Tauri projects wrap this in a recipe (e.g. `just android-debug-attach`).

### CDP eval script (≤30 lines)

```js
import WebSocket from "ws";
const list = await fetch("http://localhost:9222/json").then((r) => r.json());
const target = list.find((t) => t.type === "page");
const ws = new WebSocket(target.webSocketDebuggerUrl);
ws.on("open", () =>
  ws.send(
    JSON.stringify({
      id: 1,
      method: "Runtime.evaluate",
      params: {
        expression: process.argv[2],
        returnByValue: true,
        awaitPromise: true,
      },
    }),
  ),
);
ws.on("message", (m) => {
  console.log(JSON.stringify(JSON.parse(m).result?.result?.value));
  ws.close();
});
```

Usage:

```sh
node scripts/cdp.mjs "document.querySelector('header').getBoundingClientRect()"
node scripts/cdp.mjs "getComputedStyle(document.body).fontFamily"
node scripts/cdp.mjs "document.querySelectorAll('[data-testid]').length"
node scripts/cdp.mjs "(async () => (await fetch('/health')).status)()"
node scripts/cdp.mjs "Object.keys(window.__TAURI_INTERNALS__ ?? {})"
```

Wrap multi-statement code in an IIFE; use `awaitPromise: true` for async.

> **Known gotcha:** `chrome-remote-interface` against Android WebViews occasionally hits "socket hangup" after long idle (cyrus-and/chrome-remote-interface#562, #474). Reconnect on error rather than holding a single long-lived socket.

### Logcat triage

```sh
adb logcat -c                       # clear ring buffer before reproducing
adb logcat -d -t 500                # last 500 lines after repro
adb logcat -v time *:E              # all errors, live
```

**Tags worth filtering for Tauri apps:**

| Tag                | What it carries                                             |
| ------------------ | ----------------------------------------------------------- |
| `RustStdoutStderr` | All Rust `println!` / `eprintln!` / `log::*` / panics.      |
| `chromium`         | WebView lifecycle, network, JS engine errors.               |
| `Console`          | `console.log/warn/error` from the page.                     |
| `AndroidRuntime`   | Java/Kotlin uncaught exceptions.                            |
| `tombstoned`       | Native crashes (segfault in a `.so`); pair with tombstone.  |
| `DEBUG` / `libc`   | Native crash signal info (SIGSEGV/SIGABRT + register dump). |

Targeted view (silence everything else with `*:S`):

```sh
adb logcat -v time RustStdoutStderr:V chromium:E Console:V *:S
```

**Known noise to drop** when reading logs: `reqwest`/`hyper` connect chatter, `HwcComposer`, `SurfaceFlinger`, `BufferQueue`, `ViewRootImpl`, `setRequestedFrameRate`, `SemGameManager`. None indicate a real bug.

### HMR dev loop on Android

`tauri android dev` keeps the WebView pointed at the Vite/Webpack/etc. dev server. Three-process layout that survives Rust rebuilds:

1. **HMR session** (user terminal): `tauri android dev` (or `--no-watch` to disable Rust auto-rebuild). Vue/TS/CSS edits push live. **Pick the recipe by transport before you start** — switching `--host` ↔ USB mid-session leaves the WebView dialing a dead HMR endpoint until reloaded.
2. **CDP attach** (one-shot): `adb forward tcp:9222 …`, then `node scripts/cdp.mjs "<expr>"` for live JS eval.
3. **Rust rebuild on demand** (second terminal): rebuild + `adb install -r` to preserve app data. HMR keeps streaming; the WebView reloads and CDP retries automatically.

Transport:

| Mode           | Recipe                     | Networking                                                    |
| -------------- | -------------------------- | ------------------------------------------------------------- |
| USB / emulator | `tauri android dev`        | `adb reverse tcp:<port>` so device hits `localhost:<port>`.   |
| Wi-Fi (no USB) | `tauri android dev --host` | Sets `TAURI_DEV_HOST=<LAN IP>`, HMR over `ws://<LAN>:<port>`. |

Vite config must read `TAURI_DEV_HOST`:

```ts
const host = process.env.TAURI_DEV_HOST;
export default defineConfig({
  server: {
    host: host || false,
    port: 1420,
    strictPort: true,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
  },
});
```

Firewall must allow inbound TCP on the dev + HMR ports for `--host` mode.

### Native crash forensics

```sh
adb shell ls /data/tombstones      # requires root on most devices
adb bugreport bugreport.zip        # works without root; tombstones inside
```

Without root, the logcat `DEBUG` tag has the abbreviated stack: PC, register state, `.so` mapping. Symbolize with:

```sh
$ANDROID_NDK/toolchains/llvm/prebuilt/<host>/bin/llvm-addr2line -e <unstripped.so> <pc>
```

The unstripped `.so` lives under `target/<triple>/{debug,release}/deps/`, **not** the stripped one bundled in the APK.

### Common Android failures

| Symptom                                                              | First check                                                                                                                                                                                                                                                                                      |
| -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------ |
| `chrome://inspect` shows no targets                                  | `adb forward` not run, wrong PID suffix, or release build with debugging off.                                                                                                                                                                                                                    |
| WebView blank on launch                                              | `adb logcat chromium:E` for net/CSP errors; verify `devUrl` reachable from device browser.                                                                                                                                                                                                       |
| HMR connects then disconnects                                        | LAN mode firewall blocks 1421, or device suspended Wi-Fi.                                                                                                                                                                                                                                        |
| HMR WS times out on `ws://<LAN>:1421` after switching `--host` → USB | Stale page baked the LAN HMR config from the prior session; the new USB-mode dev server serves a different config but the WebView never refetched. `node scripts/cdp.mjs "location.reload()"` (or `adb install -r` the fresh debuggable build). Confirm with `curl -s http://localhost:9222/json | jq -r '.[0].url'`→ should be`http://tauri.localhost/`, then check `tmp/error-monitor.log` has no `1421` WS errors. |
| `UnsatisfiedLinkError` for a `.so`                                   | ABI mismatch — APK has `arm64-v8a`, device is `armeabi-v7a`, or vice versa.                                                                                                                                                                                                                      |
| Sidecar exec returns ENOENT                                          | `tauri-plugin-shell` sidecars don't run in Android sandbox; needs in-process port.                                                                                                                                                                                                               |
| Rust changes don't appear after rebuild                              | `--no-watch` is on; `adb install -r` manually.                                                                                                                                                                                                                                                   |

---

## iOS (brief)

- **Inspector:** Safari Web Inspector. Safari → Settings → Advanced → enable **Show features for web developers** (gives the Develop menu). On a physical device: Settings → Safari → Advanced → toggle **Web Inspector**.
- Run `tauri ios dev`, then in Safari → **Develop** menu → device/simulator entry → **localhost**.
- Physical device needs `TAURI_DEV_HOST` set (via Xcode network device, run `tauri ios dev --force-ip-prompt`).
- **Known issue (tauri#13346):** Safari Web Inspector can be flaky against iOS Tauri apps; restart Safari/Simulator. **Develop > Inspect Simulator** is reportedly more reliable than picking the device entry.
- First launch triggers a network permission prompt (dev server access); allow + restart the app.

---

## Logging — `tauri-plugin-log`

The official cross-platform logger. Works on Windows, Linux, macOS, **Android, iOS**. Requires Rust 1.77.2+.

```rust
use tauri_plugin_log::{Target, TargetKind};

tauri::Builder::default()
    .plugin(
        tauri_plugin_log::Builder::new()
            .target(Target::new(TargetKind::Stdout))
            .target(Target::new(TargetKind::LogDir { file_name: Some("app".into()) }))
            .target(Target::new(TargetKind::Webview))
            .level(log::LevelFilter::Info)
            .level_for("my_crate::commands", log::LevelFilter::Trace)
            .filter(|m| m.target() != "hyper")
            .build()
    );
```

```ts
import { warn, debug, info, error, attachConsole } from "@tauri-apps/plugin-log";
const detach = await attachConsole(); // Rust logs into JS console (needs Webview target)
info("hello from JS");
```

**LogDir target paths:**

| Platform | Path                                                                    |
| -------- | ----------------------------------------------------------------------- |
| Linux    | `$XDG_DATA_HOME/{bundleIdentifier}/logs` (or `~/.local/share/.../logs`) |
| macOS    | `~/Library/Logs/{bundleIdentifier}`                                     |
| Windows  | `%LOCALAPPDATA%\{bundleIdentifier}\logs`                                |

Rotation:

```rust
.max_file_size(50_000)
.rotation_strategy(tauri_plugin_log::RotationStrategy::KeepAll)
```

**Capabilities:** the JS API is gated. Add `"log:default"` to the relevant capability:

```json
{ "identifier": "main", "windows": ["main"], "permissions": ["log:default"] }
```

**Forward `console.*` into the plugin** (single source of truth):

```ts
import { warn, debug, trace, info, error } from "@tauri-apps/plugin-log";
function forward(name, logger) {
  const o = console[name];
  console[name] = (m) => {
    o(m);
    logger(m);
  };
}
forward("log", trace);
forward("debug", debug);
forward("info", info);
forward("warn", warn);
forward("error", error);
```

---

## CrabNebula DevTools — `tauri-plugin-devtools`

A purpose-built Tauri inspector: **Console** (logs/warnings/errors), **Calls** (every IPC command with args, return, perf breakdown), **Config Viewer**.

```rust
fn main() {
    #[cfg(debug_assertions)]
    let devtools = tauri_plugin_devtools::init();

    let mut builder = tauri::Builder::default();
    #[cfg(debug_assertions)] { builder = builder.plugin(devtools); }
    builder.run(tauri::generate_context!()).expect("run");
}
```

```toml
[dependencies]
tauri-plugin-devtools = "2"
```

Open the URL printed at startup, or visit `https://devtools.crabnebula.dev` and connect.

**Conflicts with `tauri-plugin-log`:** both register a global `tracing` subscriber → panic `attempted to set a logger after the logging system was already initialized`. Two fixes:

1. Use devtools in debug, plugin-log in release (`#[cfg(debug_assertions)]` gating).
2. Pipe plugin-log through devtools via `attach_logger`:
   ```rust
   let (plugin_log, max_level, logger) = tauri_plugin_log::Builder::new().split(app.handle())?;
   #[cfg(debug_assertions)]
   {
       let mut b = tauri_plugin_devtools::Builder::default();
       b.attach_logger(logger);
       app.handle().plugin(b.init())?;
   }
   #[cfg(not(debug_assertions))]
   tauri_plugin_log::attach_logger(max_level, logger);
   app.handle().plugin(plugin_log)?;
   ```

**Android emulator:** the WebSocket server (port 3033) is behind the emulator's NAT. Forward it:

```sh
adb forward tcp:3033 tcp:3033
```

---

## IPC / capability debugging

Tauri 2 IPC is **deny-by-default**. Every plugin command needs a permission grant in `src-tauri/capabilities/*.json`.

Typical runtime error in the WebView console:

```
<command> not allowed. Permissions associated with this command: …
```

Fix: add the listed permission identifier to a capability that targets the calling window:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "main",
  "windows": ["main"],
  "permissions": ["core:default", "log:default", "shell:allow-open"]
}
```

**Command serialization gotchas:**

- All command args must implement `serde::Deserialize`. JS sends camelCase keys; Rust uses snake_case fields → use `#[serde(rename_all = "camelCase")]` on structs, **or** name Rust params in camelCase (Tauri auto-converts).
- All returns must implement `serde::Serialize`, **including errors**. Use a `thiserror` enum with a manual `Serialize` impl that emits `{ kind, message }`:
  ```rust
  #[derive(Debug, thiserror::Error)]
  enum Error { #[error(transparent)] Io(#[from] std::io::Error) }
  impl serde::Serialize for Error {
      fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
          s.serialize_str(&self.to_string())
      }
  }
  ```
- Large returns (megabytes): use `tauri::ipc::Response::new(Vec<u8>)` to skip JSON. Streams: use `tauri::ipc::Channel`.
- `tauri::generate_handler![...]` accepts an array. **Calling `.invoke_handler` twice silently keeps only the last call** — pass every command in one call.

---

## Environment variables (Tauri 2 reference)

| Var                                  | Meaning                                                       |
| ------------------------------------ | ------------------------------------------------------------- |
| `TAURI_ENV_PLATFORM`                 | `windows` / `darwin` / `linux` / `android` / `ios`            |
| `TAURI_ENV_ARCH`                     | `x86_64` / `aarch64` / …                                      |
| `TAURI_ENV_FAMILY`                   | `unix` / `windows`                                            |
| `TAURI_ENV_DEBUG`                    | `true` for `dev` and `build --debug`                          |
| `TAURI_ENV_TARGET_TRIPLE`            | Target triple                                                 |
| `TAURI_DEV_HOST`                     | Address dev server should listen on (mobile, esp. iOS device) |
| `TAURI_CLI_PORT`                     | Override dev server port (was `TAURI_DEV_SERVER_PORT`)        |
| `TAURI_CLI_NO_DEV_SERVER_WAIT`       | Skip wait for dev server (was `TAURI_SKIP_DEVSERVER_CHECK`)   |
| `TAURI_CLI_CONFIG_DEPTH`             | Config search depth (was `TAURI_PATH_DEPTH`)                  |
| `TAURI_SIGNING_PRIVATE_KEY`          | Updater signing key (was `TAURI_PRIVATE_KEY`)                 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Key password (was `TAURI_KEY_PASSWORD`)                       |
| `RUST_LOG`                           | e.g. `tauri=debug,wry=debug,tao=debug`                        |
| `RUST_BACKTRACE`                     | `1` or `full` for panics                                      |

---

## Diagnostic checklist

Before guessing, in this order:

1. **Process actually running?** `adb shell pidof <pkg>` / Task Manager / `pgrep`.
2. **DevTools/CDP reachable?** `curl http://localhost:9222/json` → ≥1 target.
3. **What does the _last_ error in logs say?** Not the first — Tauri prints init noise.
4. **Frontend or backend?** A 10-second CDP narrows it:
   ```
   node scripts/cdp.mjs "({err: window.__lastError, tauri: !!window.__TAURI_INTERNALS__, perms: typeof __TAURI__})"
   ```
5. **Reproduce with logs cleared** (`adb logcat -c`, restart `tauri dev`) so the trail is short.
6. **IPC error?** Read the _exact_ permission identifier from the message and grep `src-tauri/capabilities/`.

## Anti-patterns

- **Don't** screenshot to read text or measure layout — use CDP `getBoundingClientRect`/`getComputedStyle`/`outerHTML`.
- **Don't** dump full logcat into agent context — filter by tag, drop known noise, return only the failing window.
- **Don't** ship release builds with the `devtools` Cargo feature.
- **Don't** assume CDP is attached after a Rust rebuild — `curl http://localhost:9222/json` confirms in 1 second.
- **Don't** call `.invoke_handler` more than once — silently overrides previous calls.
- **Don't** mix `tauri-plugin-log` and `tauri-plugin-devtools` in the same build mode without `attach_logger` — they fight over the global tracing subscriber.
- **Don't** grep `target/` for stripped `.so`s when symbolizing native crashes — use the unstripped copies under `target/<triple>/{debug,release}/deps/`.

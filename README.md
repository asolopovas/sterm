# sterm

**sterm** is a simple remote terminal app.

Run the app on your desktop, scan a QR code with the Android app, and use your desktop shell from your phone.

It is built with:

- **Tauri 2** for the desktop/mobile app shell
- **Rust** for the host, networking, terminal handling, and protocol code
- **Vue + xterm.js** for the UI and terminal view
- **QUIC** for encrypted network connections

> ⚠️ **Security note:** this gives another device access to a terminal on your computer. Only pair with devices you trust. Do not share pairing QR codes, tokens, or relay/tracker details publicly.

## What it does

- Starts a terminal host on the desktop.
- Shows a QR code / pairing JSON.
- Lets the Android app connect to the desktop.
- Opens an interactive terminal session from the phone.
- Supports direct LAN, tracker-assisted discovery, and relay-based connection modes.

## How to use it

### 1. Start the desktop app

```bash
npm install
npm run tauri dev
```

The desktop app starts a background host service and shows pairing details.

### 2. Choose a connection mode

You can pick one of three modes:

| Mode | Use this when | Notes |
| --- | --- | --- |
| **Direct LAN** | Phone and desktop are on the same Wi-Fi/LAN | Fastest and simplest |
| **Tracker P2P discovery** | Both devices can reach each other, but you want discovery help | Uses a UDP BitTorrent-style tracker to find the host |
| **Relay rendezvous** | Direct connection does not work, e.g. NAT/cellular/firewall problems | Traffic goes through a relay |

Direct mode is usually the easiest place to start.

### 3. Pair the Android app

On Android:

1. Open the app.
2. Scan the QR code from the desktop app, or paste the pairing JSON.
3. Enter the optional password if the desktop host requires one.
4. Open the terminal session.

## Connection details

The desktop host always tries to keep direct QUIC listening available.

By default it tries UDP port `4433`. If that port is unavailable, it falls back to a random available UDP port.

If the phone cannot connect:

- Make sure both devices are on the same network for **Direct LAN**.
- Allow the app through your OS firewall, especially on private networks.
- Try **Relay rendezvous** if you are on cellular, public Wi-Fi, or behind strict NAT.

## Development setup

You need:

- Node.js and npm
- Rust via `rustup`
- Tauri 2 prerequisites for your OS
- For Android work: Android Studio/SDK, NDK, and `adb`

Install dependencies:

```bash
npm install
```

Run the desktop dev app:

```bash
npm run tauri dev
```

If you use [`just`](https://github.com/casey/just), common commands are available:

```bash
just doctor       # check required tools
just dev          # run desktop dev app
just check        # run frontend + Rust checks
just build        # build packaged desktop app
just clean        # remove build outputs
```

## Useful development commands

Frontend build/type check:

```bash
npm run build
```

Rust check:

```bash
cd src-tauri
cargo check
```

Rust clippy:

```bash
cd src-tauri
cargo clippy --all-targets -- -D warnings
```

Run all essential checks:

```bash
just check
```

## Android development

Build a debug APK for a connected Android device:

```bash
just android-build
```

Install it:

```bash
just android-install
```

Launch it:

```bash
just android-run
```

Build, install, and launch:

```bash
just install
```

## Standalone CLI host

There is also a standalone host binary for testing:

```bash
just host-build
just host
```

Example with a specific shell and password:

```bash
just host ARGS='--shell wsl --password secret'
```

## Project layout

```text
src/                         Vue frontend
src-tauri/                   Tauri/Rust app
src-tauri/src/               App backend: host, client, pairing, terminal logic
src-tauri/crates/            Reused protocol/CLI crates
src-tauri/crates/rterm-protocol  Shared auth, frames, TLS pins, tracker logic
scripts/                     Dev/debug helper scripts
```

## Notes for contributors

- Keep pairing secrets out of commits and logs.
- Prefer `just check` before pushing changes.
- If connection tests fail, check firewall rules and UDP reachability first.
- Relay mode is the fallback when direct UDP cannot work.

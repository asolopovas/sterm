# sterm

A simple Rust/Tauri remote terminal app built by nesting the working `tsync/rterm` codebase.

- Desktop app: starts a background QUIC host service and shows pairing details.
- Android app: scans/pastes pairing JSON and opens a terminal session.
- Reused code is nested under `src-tauri/crates/`:
  - `rterm-protocol` is used directly for frames, auth, TLS pins, tracker discovery, and token rules.
  - `rterm-host`, `rterm-client`, and `rterm-relay` are carried into the app tree as the source foundation.
- The Tauri host/client modules adapt the same direct, tracker, and relay flows for IPC/UI use.

## Connection modes

The desktop host always keeps direct QUIC listening available.

From the host screen you can choose:

1. **Direct LAN** — QR contains `host`, `token`, and direct host certificate pin.
2. **Tracker P2P discovery** — QR contains the UDP BitTorrent-style tracker and room. The host announces to the tracker and the Android client asks the tracker for peers, then connects directly to the host over QUIC.
3. **Relay rendezvous** — QR contains the relay address. The desktop registers as host with the relay and the Android client connects through the relay pairing flow.

Tracker mode still requires the phone to be able to reach the discovered host UDP port. Use relay rendezvous when NAT/cellular prevents direct UDP reachability.

## Development

```bash
npm install
npm run tauri dev
```

The desktop host listens on UDP port `4433` when available and falls back to an ephemeral UDP port. Allow the app through the OS firewall on private networks.

## Pairing

1. Run the desktop app.
2. Pick direct, tracker P2P, or relay rendezvous.
3. Scan the QR code from Android, or copy/paste the pairing JSON.
4. Open the terminal session.

## Checks

```bash
npm run build
cd src-tauri && cargo check
cd src-tauri && cargo clippy --all-targets -- -D warnings
```

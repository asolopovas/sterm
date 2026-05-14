# AGENTS.md

Coding-agent rules for this repo.

## Project

`sterm` is a Tauri 2 remote-terminal app:

- `src/`: Vue 3, TypeScript, Vite, xterm.js
- `src-tauri/src/`: Rust/Tauri app code
- `src-tauri/crates/`: shared Rust protocol/tools
- `src-tauri/gen/`: generated mobile project files

Treat pairing data, passwords, tokens, relay/tracker details, QR payloads, and terminal contents as secrets.

## Rules

- Do not add code comments unless absolutely necessary.
- Run all checks before committing and pushing.

## Commands

Prefer `just`:

- `just bootstrap`: install dependencies
- `just dev`: run desktop dev app
- `just check`: frontend typecheck + Rust check
- `just verify`: full validation
- `just build-debug`: debug build
- `just build`: release/package build
- `just host "--help"`: host CLI
- `just doctor android`, `just android-dev`, `just android-build`: Android
- `just clean`: clean outputs

## Validation

Run the narrowest useful check before finishing:

- Frontend: `bun run typecheck`
- Rust: `cargo check --manifest-path src-tauri/Cargo.toml`
- Cross-cutting: `just check`
- Larger changes: `just verify`

Report what you ran and what you skipped.

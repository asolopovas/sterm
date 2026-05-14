# Cross-platform command runner for sterm.
# Official just guidance: use `windows-shell` instead of forcing Bash on Windows.
set windows-shell := ["cmd.exe", "/C"]

cargo_manifest := "src-tauri/Cargo.toml"
android_target := env_var_or_default("ANDROID_TARGET", "aarch64")
android_apk := "src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk"
app_id := "com.local.sterm"

# List the essential project commands
_default:
    just --list

# Install dependencies from bun.lock
bootstrap:
    bun install --frozen-lockfile

# Check required tools. Use `just doctor android` for Android tools.
doctor TARGET="":
    bun scripts/doctor.mjs {{TARGET}}

# Start the desktop Tauri dev app
dev:
    bun tauri dev

# Start desktop dev app with WebView CDP debugging on port 9223
dev-debug:
    bun scripts/dev-debug.mjs

# Fast local check: frontend types + Rust check
check: _frontend-check _rust-check

# Full pre-commit verification: check + formatting + clippy + tests
verify: check _fmt-check _clippy _rust-test

# Build fast debug artifacts without packaging installers
build-debug: _frontend-build
    cargo build --manifest-path {{cargo_manifest}}

# Build the desktop Tauri release package
build: doctor
    bun tauri build

# Run the standalone CLI host. Example: just host "--shell wsl --password secret"
host ARGS="":
    cargo run --manifest-path {{cargo_manifest}} --bin sterm-host -- {{ARGS}}

# Start Android dev app on a connected device/emulator
android-dev: _android-doctor
    bun tauri android dev --target {{android_target}}

# Build Android debug APK. Override target: ANDROID_TARGET=armv7 just android-build
android-build: _android-doctor
    bun tauri android build --debug --target {{android_target}}

# Build, install, and launch Android app
install: android-build
    adb install -r {{android_apk}}
    adb shell monkey -p {{app_id}} -c android.intent.category.LAUNCHER 1

# Forward Android WebView DevTools to localhost:9222
android-debug: _android-doctor
    bun scripts/android-debug.mjs {{app_id}}

# Launch Android app and save a PNG screenshot under tmp/
android-snapshot: _android-doctor
    bun scripts/android-snapshot.mjs {{app_id}} tmp

# Remove generated build outputs
clean:
    bun scripts/clean.mjs

# CI-style validation for this repository
ci: bootstrap verify build-debug

_frontend-check:
    bun run typecheck

_frontend-build:
    bun run build

_rust-check:
    cargo check --manifest-path {{cargo_manifest}}

_fmt-check:
    cargo fmt --manifest-path {{cargo_manifest}} --all -- --check

_clippy:
    cargo clippy --manifest-path {{cargo_manifest}} --all-targets -- -D warnings

_rust-test:
    cargo test --manifest-path {{cargo_manifest}}

_android-doctor:
    bun scripts/doctor.mjs android

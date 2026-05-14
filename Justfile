# Cross-platform command runner for sterm.
# Official just guidance: use `windows-shell` instead of forcing Bash on Windows.
set windows-shell := ["cmd.exe", "/C"]

cargo_manifest := "src-tauri/Cargo.toml"
android_target := env_var_or_default("ANDROID_TARGET", "aarch64")
android_apk := "src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk"
app_id := "com.local.sterm"

# List available tasks
_default:
    just --list

# Install JavaScript dependencies exactly from bun.lock
bootstrap:
    bun install --frozen-lockfile

# Print required desktop tool versions
doctor:
    bun scripts/doctor.mjs

# Print Android tool versions and fail early if adb is missing
android-doctor:
    bun scripts/doctor.mjs android

# Fast development check: TypeScript + Rust type checking
check: frontend-check rust-check

# Full local verification before committing
verify: check lint rust-test

# Alias for full local verification
test: verify

# Run frontend type checking only
frontend-check:
    bun run typecheck

# Build frontend assets
frontend-build:
    bun run build

# Start frontend dev server
frontend-dev:
    bun run dev

# Preview frontend build
frontend-preview:
    bun run preview

# Run cargo check for the Tauri crate
rust-check:
    cargo check --manifest-path {{cargo_manifest}}

# Check Rust formatting without modifying files
fmt-check:
    cargo fmt --manifest-path {{cargo_manifest}} --all -- --check

# Format Rust sources
fmt:
    cargo fmt --manifest-path {{cargo_manifest}} --all

# Run clippy with warnings as errors
clippy:
    cargo clippy --manifest-path {{cargo_manifest}} --all-targets -- -D warnings

# Run Rust tests
rust-test:
    cargo test --manifest-path {{cargo_manifest}}

# Run all lint tasks
lint: fmt-check clippy

# Build fast debug artifacts without packaging installers
build-debug: frontend-build
    cargo build --manifest-path {{cargo_manifest}}

# Build the desktop Tauri app package (release installers/bundles)
build: doctor
    bun tauri build

# Start desktop Tauri dev app
dev:
    bun tauri dev

# Start desktop Tauri dev app with WebView CDP debugging on port 9223
dev-debug:
    bun scripts/dev-debug.mjs

# Build the standalone CLI host
host-build:
    cargo build --manifest-path {{cargo_manifest}} --bin sterm-host

# Run CLI host. Example: just host "--shell wsl --password secret"
host ARGS="":
    cargo run --manifest-path {{cargo_manifest}} --bin sterm-host -- {{ARGS}}

# Start Android dev app on a connected device/emulator
android-dev: android-doctor
    bun tauri android dev --target {{android_target}}

# Build Android debug APK. Override target: ANDROID_TARGET=armv7 just android-build
android-build: android-doctor
    bun tauri android build --debug --target {{android_target}}

# Install Android debug APK on connected device
android-install: android-build
    adb install -r {{android_apk}}

# Launch installed Android app on connected device
android-run: android-doctor
    adb shell monkey -p {{app_id}} -c android.intent.category.LAUNCHER 1

# Build, install, and launch Android app
install: android-install android-run

# Forward Android WebView DevTools to localhost:9222
android-debug: android-doctor
    bun scripts/android-debug.mjs {{app_id}}

# Remove generated build outputs
clean:
    bun scripts/clean.mjs

# CI-style validation for this repository
ci: bootstrap verify build-debug

set shell := ["bash", "-cu"]

# VS Code may launch `just` from PowerShell with a reduced Windows PATH. Export a
# Windows-style PATH so Git Bash converts these entries to /c/... paths on startup.
USER_HOME := env_var_or_default('USERPROFILE', env_var_or_default('HOME', ''))
LOCAL_APPDATA := env_var_or_default('LOCALAPPDATA', if USER_HOME != '' { USER_HOME + '\AppData\Local' } else { '' })
APPDATA_DIR := env_var_or_default('APPDATA', if USER_HOME != '' { USER_HOME + '\AppData\Roaming' } else { '' })
NODE_DIR := env_var_or_default('ProgramFiles', 'C:\Program Files') + '\nodejs'
CARGO_DIR := if USER_HOME != '' { USER_HOME + '\.cargo\bin' } else { '' }
WINGET_DIR := if LOCAL_APPDATA != '' { LOCAL_APPDATA + '\Microsoft\WinGet\Links' } else { '' }
WINDOWS_APPS_DIR := if LOCAL_APPDATA != '' { LOCAL_APPDATA + '\Microsoft\WindowsApps' } else { '' }
NPM_GLOBAL_DIR := if APPDATA_DIR != '' { APPDATA_DIR + '\npm' } else { '' }
ANDROID_SDK_DIR := env_var_or_default('ANDROID_HOME', if LOCAL_APPDATA != '' { LOCAL_APPDATA + '\Android\Sdk' } else { '' })
ANDROID_PLATFORM_TOOLS_DIR := if ANDROID_SDK_DIR != '' { ANDROID_SDK_DIR + '\platform-tools' } else { '' }
export PATH := env_var('PATH') + ';' + NODE_DIR + (if CARGO_DIR != '' { ';' + CARGO_DIR } else { '' }) + (if WINGET_DIR != '' { ';' + WINGET_DIR } else { '' }) + (if WINDOWS_APPS_DIR != '' { ';' + WINDOWS_APPS_DIR } else { '' }) + (if NPM_GLOBAL_DIR != '' { ';' + NPM_GLOBAL_DIR } else { '' }) + (if ANDROID_PLATFORM_TOOLS_DIR != '' { ';' + ANDROID_PLATFORM_TOOLS_DIR } else { '' })

# List available tasks
_default:
    just --list

# Print core tool versions and fail early if required desktop build tools are missing
doctor:
    command -v node npm npx cargo rustc >/dev/null
    node --version
    npm --version
    cargo --version
    rustc --version

# Print Android tool versions and fail early if adb is missing
android-doctor: doctor
    command -v adb >/dev/null
    adb version | head -1

# Run essential frontend and Rust checks
check: doctor
    npm run build
    cd src-tauri && cargo check
    cd src-tauri && cargo clippy --all-targets -- -D warnings

# Alias for check
test: check

# Build the desktop Tauri app package (includes frontend and Rust backend)
build: doctor
    npm run tauri build

# Build fast debug artifacts without packaging installers
build-debug: doctor
    npm run build
    cd src-tauri && cargo build

# Start desktop Tauri dev app
dev: doctor
    npm run tauri dev

# Start desktop Tauri dev app with WebView CDP debugging on port 9223
dev-debug: doctor
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9223' RUST_BACKTRACE=1 npm run tauri dev

# Build the standalone CLI host
host-build: doctor
    cd src-tauri && cargo build --bin sterm-host

# Run CLI host. Override ARGS, e.g. just host ARGS='--shell wsl --password secret'
host ARGS="": doctor
    cd src-tauri && cargo run --bin sterm-host -- {{ARGS}}

# Build Android debug APK for the connected device architecture
android-build: android-doctor
    npx tauri android build --debug --target aarch64

# Install Android debug APK on connected device
android-install: android-build
    adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk

# Launch Android app on connected device
android-run: android-doctor
    adb shell monkey -p com.local.sterm -c android.intent.category.LAUNCHER 1

# Build, install, and launch Android app
install: android-install android-run

# Forward Android WebView DevTools to localhost:9222
android-debug: android-doctor
    PID=$(adb shell pidof com.local.sterm | tr -d '\r'); test -n "$PID"; adb forward --remove tcp:9222 || true; adb forward tcp:9222 localabstract:webview_devtools_remote_$PID; curl -s http://127.0.0.1:9222/json

# Remove generated build outputs
clean: doctor
    rm -rf dist
    cd src-tauri && cargo clean

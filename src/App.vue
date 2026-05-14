<script setup lang="ts">
import { Channel } from "@tauri-apps/api/core";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { computed, nextTick, onMounted, onUnmounted, ref } from "vue";
import {
  availableTerminals,
  connectTerminal,
  disconnectTerminal,
  enableRelayPairing,
  enableTrackerPairing,
  runtimeInfo,
  sendTerminalInput,
  serviceInfo,
  testInternetP2p,
  updateHostSettings,
} from "./api";
import type {
  HostServiceInfo,
  PairingPayload,
  RuntimeInfo,
  TerminalEvent,
  TerminalOption,
  TerminalShell,
} from "./types";

const runtime = ref<RuntimeInfo | null>(null);
const hostInfo = ref<HostServiceInfo | null>(null);
const status = ref("Starting…");
const error = ref("");
const pairingText = ref("");
const trackerAddress = ref("");
const trackerRoom = ref("default");
const relayAddress = ref("");
const relayCertSha256 = ref("");
const terminalOptions = ref<TerminalOption[]>([]);
const selectedShell = ref<TerminalShell>("powerShell");
const hostPassword = ref("");
const pairingPassword = ref("");
const probeLog = ref("");
const probeRunning = ref(false);
const terminal = ref("");
const sessionId = ref<string | null>(null);
const terminalEl = ref<HTMLElement | null>(null);
let xterm: Terminal | null = null;
let fitAddon: FitAddon | null = null;
let resizeObserver: ResizeObserver | null = null;
const videoEl = ref<HTMLVideoElement | null>(null);
const screen = ref<"connect" | "scanner" | "terminal">("connect");
const scannerMessage = ref("Align the QR code within the frame to connect.");
const keyboardInset = ref(0);
let scannerStream: MediaStream | null = null;
let scannerTimer: number | null = null;

type BarcodeDetectorLike = {
  detect(source: CanvasImageSource): Promise<Array<{ rawValue: string }>>;
};

type BarcodeDetectorConstructor = new (options?: { formats?: string[] }) => BarcodeDetectorLike;

const isDesktopHost = computed(() => runtime.value?.isDesktopHost ?? false);
const connected = computed(() => Boolean(sessionId.value));

onMounted(async () => {
  setupViewportInsets();
  runtime.value = await runtimeInfo();
  if (runtime.value.isDesktopHost) {
    terminalOptions.value = await availableTerminals();
    await refreshServiceInfo();
    selectedShell.value = hostInfo.value?.shell ?? "powerShell";
  } else {
    status.value = "Not connected";
  }
});

onUnmounted(() => {
  window.visualViewport?.removeEventListener("resize", updateViewportInsets);
  window.visualViewport?.removeEventListener("scroll", updateViewportInsets);
  stopScanner();
  disposeTerminal();
});

function setupViewportInsets() {
  window.visualViewport?.addEventListener("resize", updateViewportInsets);
  window.visualViewport?.addEventListener("scroll", updateViewportInsets);
  updateViewportInsets();
}

function updateViewportInsets() {
  const viewport = window.visualViewport;
  const inset = viewport
    ? Math.max(0, window.innerHeight - viewport.height - viewport.offsetTop)
    : 0;
  keyboardInset.value = Math.round(inset);
  document.documentElement.style.setProperty("--keyboard-inset", `${keyboardInset.value}px`);
  requestAnimationFrame(() => fitAddon?.fit());
}

async function refreshServiceInfo() {
  error.value = "";
  for (let i = 0; i < 30; i += 1) {
    try {
      hostInfo.value = await serviceInfo();
      status.value = hostInfo.value.status;
      return;
    } catch (err) {
      status.value = String(err);
      await new Promise((resolve) => setTimeout(resolve, 300));
    }
  }
}

async function copyPairing() {
  if (!hostInfo.value) return;
  await navigator.clipboard.writeText(hostInfo.value.pairingJson);
  status.value = "Pairing code copied";
}

async function useTrackerPairing() {
  error.value = "";
  try {
    hostInfo.value = await enableTrackerPairing(trackerAddress.value, trackerRoom.value);
    status.value = hostInfo.value.status;
  } catch (err) {
    error.value = String(err);
  }
}

async function useRelayPairing() {
  error.value = "";
  try {
    hostInfo.value = await enableRelayPairing(
      relayAddress.value,
      relayCertSha256.value || undefined,
    );
    status.value = hostInfo.value.status;
  } catch (err) {
    error.value = String(err);
  }
}

async function saveHostSettings() {
  error.value = "";
  try {
    hostInfo.value = await updateHostSettings(selectedShell.value, hostPassword.value || undefined);
    status.value = "Host settings updated";
  } catch (err) {
    error.value = String(err);
  }
}

async function runInternetP2pTest() {
  error.value = "";
  probeLog.value = "";
  probeRunning.value = true;
  const tracker =
    trackerAddress.value || hostInfo.value?.pairing.tracker || "udp://tracker.opentrackr.org:1337";
  const room = trackerRoom.value || hostInfo.value?.pairing.trackerRoom || "sterm-probe";
  const channel = new Channel<TerminalEvent>();
  channel.onmessage = (message) => {
    if (message.event === "status" || message.event === "error") {
      probeLog.value += `[${message.event}] ${message.data}\n`;
    }
  };
  try {
    await testInternetP2p(tracker, room, channel);
    probeLog.value += "[done] internet UDP P2P probe succeeded\n";
  } catch (err) {
    probeLog.value += `[failed] ${String(err)}\n`;
    error.value = String(err);
  } finally {
    probeRunning.value = false;
  }
}

function parsePairing(): PairingPayload {
  const raw = JSON.parse(pairingText.value.trim()) as Record<string, unknown>;
  const parsed = expandPairing(raw);
  if (parsed.requiresPassword) {
    if (!pairingPassword.value) throw new Error("password is required for this host");
    parsed.password = pairingPassword.value;
  }
  if (parsed.v !== 1 || !parsed.token || !parsed.certSha256) {
    throw new Error("invalid pairing code");
  }
  if (parsed.mode === "direct" && !parsed.host) throw new Error("direct pairing is missing host");
  if (parsed.mode === "tracker" && !parsed.tracker)
    throw new Error("tracker pairing is missing tracker");
  if (parsed.mode === "relay" && !parsed.relay) throw new Error("relay pairing is missing relay");
  return parsed;
}

function expandPairing(raw: Record<string, unknown>): PairingPayload {
  if (typeof raw.mode === "string") {
    return raw as unknown as PairingPayload;
  }
  const mode =
    raw.m === "d" ? "direct" : raw.m === "t" ? "tracker" : raw.m === "r" ? "relay" : undefined;
  if (!mode) throw new Error("unknown pairing mode");
  return {
    v: Number(raw.v),
    mode,
    host: typeof raw.h === "string" ? raw.h : undefined,
    token: String(raw.k ?? ""),
    certSha256: String(raw.c ?? ""),
    tracker: typeof raw.r === "string" ? raw.r : undefined,
    trackerRoom: typeof raw.q === "string" ? raw.q : undefined,
    relay: typeof raw.y === "string" ? raw.y : undefined,
    relayCertSha256: typeof raw.p === "string" ? raw.p : undefined,
    requiresPassword: raw.a === true,
  };
}

async function startTerminal() {
  stopScanner();
  error.value = "";
  terminal.value = "";
  disposeTerminal();
  try {
    const pairing = parsePairing();
    screen.value = "terminal";
    await nextTick();
    setupTerminal();

    const channel = new Channel<TerminalEvent>();
    channel.onmessage = async (message) => {
      if (message.event === "output") {
        terminal.value += message.data;
        xterm?.write(message.data);
      } else if (message.event === "status") {
        status.value = message.data;
        xterm?.writeln(`\x1b[38;5;244m[${message.data}]\x1b[0m`);
      } else if (message.event === "error") {
        error.value = message.data;
        xterm?.writeln(`\x1b[31m[error] ${message.data}\x1b[0m`);
      } else if (message.event === "closed") {
        status.value = "Disconnected";
        sessionId.value = null;
        xterm?.writeln("\r\n\x1b[33m[disconnected]\x1b[0m");
      }
      await nextTick();
    };
    sessionId.value = await connectTerminal(pairing, channel);
    xterm?.focus();
  } catch (err) {
    error.value = String(err);
    screen.value = "connect";
  }
}

function setupTerminal() {
  if (!terminalEl.value) return;
  fitAddon = new FitAddon();
  xterm = new Terminal({
    cursorBlink: true,
    convertEol: true,
    fontFamily: "Cascadia Mono, Consolas, 'Courier New', monospace",
    fontSize: 14,
    lineHeight: 1.15,
    scrollback: 5000,
    theme: {
      background: "#0c0c0c",
      foreground: "#cccccc",
      cursor: "#ffffff",
      selectionBackground: "#264f78",
      black: "#0c0c0c",
      red: "#c50f1f",
      green: "#13a10e",
      yellow: "#c19c00",
      blue: "#0037da",
      magenta: "#881798",
      cyan: "#3a96dd",
      white: "#cccccc",
      brightBlack: "#767676",
      brightRed: "#e74856",
      brightGreen: "#16c60c",
      brightYellow: "#f9f1a5",
      brightBlue: "#3b78ff",
      brightMagenta: "#b4009e",
      brightCyan: "#61d6d6",
      brightWhite: "#f2f2f2",
    },
  });
  xterm.loadAddon(fitAddon);
  xterm.open(terminalEl.value);
  fitAddon.fit();
  xterm.onData((data) => {
    void sendRaw(data);
  });
  resizeObserver = new ResizeObserver(() => fitAddon?.fit());
  resizeObserver.observe(terminalEl.value);
}

function disposeTerminal() {
  resizeObserver?.disconnect();
  resizeObserver = null;
  xterm?.dispose();
  xterm = null;
  fitAddon = null;
}

async function sendRaw(data: string) {
  if (!sessionId.value) return;
  await sendTerminalInput(sessionId.value, data);
}

function focusTerminal() {
  xterm?.focus();
}

async function copyTerminalSelection() {
  const selected = xterm?.getSelection();
  const text = selected && selected.length > 0 ? selected : terminal.value;
  if (!text) return;
  await navigator.clipboard.writeText(text);
  status.value = selected ? "Selection copied" : "Terminal buffer copied";
}

async function openScanner() {
  error.value = "";
  screen.value = "scanner";
  await nextTick();
  await startScanner();
}

async function startScanner() {
  stopScanner();
  const Detector = (window as Window & { BarcodeDetector?: BarcodeDetectorConstructor })
    .BarcodeDetector;
  if (!Detector || !navigator.mediaDevices?.getUserMedia) {
    scannerMessage.value = "Camera QR scanning is unavailable here. Paste the pairing code below.";
    return;
  }

  try {
    scannerStream = await navigator.mediaDevices.getUserMedia({
      video: {
        facingMode: { ideal: "environment" },
        width: { ideal: 1280 },
        height: { ideal: 720 },
        frameRate: { ideal: 30 },
      },
      audio: false,
    });
    if (videoEl.value) {
      videoEl.value.srcObject = scannerStream;
      await videoEl.value.play();
    }
    const detector = new Detector({ formats: ["qr_code"] });
    scannerMessage.value = "Align the QR code within the frame to connect.";
    let detecting = false;
    const scan = async () => {
      if (!videoEl.value || screen.value !== "scanner") return;
      if (!detecting && videoEl.value.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA) {
        detecting = true;
        try {
          const codes = await detector.detect(videoEl.value);
          const rawValue = codes[0]?.rawValue;
          if (rawValue) {
            pairingText.value = rawValue;
            await startTerminal();
            return;
          }
        } finally {
          detecting = false;
        }
      }
      scannerTimer = window.requestAnimationFrame(scan);
    };
    scannerTimer = window.requestAnimationFrame(scan);
  } catch (err) {
    scannerMessage.value = `Camera unavailable: ${String(err)}. Paste the pairing code below.`;
  }
}

function stopScanner() {
  if (scannerTimer !== null) {
    window.cancelAnimationFrame(scannerTimer);
    scannerTimer = null;
  }
  scannerStream?.getTracks().forEach((track) => track.stop());
  scannerStream = null;
}

async function disconnect() {
  if (sessionId.value) {
    await disconnectTerminal(sessionId.value);
  }
  sessionId.value = null;
  disposeTerminal();
  screen.value = "connect";
  status.value = "Disconnected";
}
</script>

<template>
  <div class="app-shell" :style="{ '--keyboard-inset': `${keyboardInset}px` }">
    <header class="topbar">
      <button class="icon-button" aria-label="Remote terminal">⌘</button>
      <div>
        <h1>Remote Terminal</h1>
        <p><span :class="['dot', connected || hostInfo?.running ? 'ok' : 'idle']" />{{ status }}</p>
      </div>
      <button
        class="icon-button"
        aria-label="Refresh"
        @click="isDesktopHost ? refreshServiceInfo() : (screen = 'connect')"
      >
        ↻
      </button>
    </header>

    <main v-if="isDesktopHost" class="host-layout">
      <section class="card intro" v-if="hostInfo?.firstRun">
        <h2>Service ready</h2>
        <p>
          The desktop host service is running in the background. Keep this app open and allow UDP
          traffic on your private network if the firewall asks.
        </p>
      </section>

      <section class="card qr-card" v-if="hostInfo">
        <div class="qr" v-html="hostInfo.qrSvg" />
        <div class="details">
          <h2>Pair your Android phone</h2>
          <p>Scan this QR code from the Android client, or copy the pairing code manually.</p>
          <dl>
            <div>
              <dt>Mode</dt>
              <dd>{{ hostInfo.pairing.mode }}</dd>
            </div>
            <div v-if="hostInfo.pairing.host">
              <dt>Host</dt>
              <dd>{{ hostInfo.pairing.host }}</dd>
            </div>
            <div v-if="hostInfo.pairing.tracker">
              <dt>Tracker</dt>
              <dd>{{ hostInfo.pairing.tracker }}</dd>
            </div>
            <div v-if="hostInfo.pairing.relay">
              <dt>Relay</dt>
              <dd>{{ hostInfo.pairing.relay }}</dd>
            </div>
            <div>
              <dt>UDP listen</dt>
              <dd>{{ hostInfo.listen }}</dd>
            </div>
            <div>
              <dt>Certificate pin</dt>
              <dd>{{ hostInfo.certSha256 }}</dd>
            </div>
          </dl>
          <button class="primary" @click="copyPairing">Copy pairing code</button>
        </div>
      </section>

      <section class="card" v-if="hostInfo">
        <h2>Terminal and authentication</h2>
        <p>
          Choose which host shell new phone sessions open, and optionally require a password in
          addition to the QR pairing secret.
        </p>
        <div class="settings-grid">
          <label>
            Terminal
            <select v-model="selectedShell">
              <option
                v-for="option in terminalOptions"
                :key="option.shell"
                :value="option.shell"
                :disabled="!option.available"
              >
                {{ option.label }}{{ option.available ? "" : " (not installed)" }}
              </option>
            </select>
          </label>
          <label>
            Password before connect
            <input
              v-model="hostPassword"
              type="password"
              placeholder="optional"
              autocomplete="new-password"
              maxlength="256"
            />
          </label>
          <button class="secondary" @click="saveHostSettings">Update QR</button>
        </div>
      </section>

      <section class="card p2p-card" v-if="hostInfo">
        <h2>P2P discovery and rendezvous</h2>
        <p>
          Direct LAN stays available. Enable the original tsync BitTorrent-style UDP tracker
          discovery or relay rendezvous and the QR code updates.
        </p>
        <div class="settings-grid">
          <label>
            UDP tracker
            <input
              v-model="trackerAddress"
              placeholder="udp://tracker.example:6969"
              maxlength="256"
            />
          </label>
          <label>
            Room
            <input v-model="trackerRoom" placeholder="default" maxlength="64" />
          </label>
          <button class="secondary" @click="useTrackerPairing">Use tracker P2P</button>
        </div>
        <div class="settings-grid">
          <label>
            Relay
            <input v-model="relayAddress" placeholder="relay.example:4433" maxlength="256" />
          </label>
          <label>
            Relay cert pin (optional)
            <input v-model="relayCertSha256" placeholder="for self-signed relays" maxlength="95" />
          </label>
          <button class="secondary" @click="useRelayPairing">Use relay rendezvous</button>
        </div>
        <button class="primary" :disabled="probeRunning" @click="runInternetP2pTest">
          {{ probeRunning ? "Testing…" : "Test internet P2P first" }}
        </button>
        <p class="helper">
          Run this on desktop and Android at the same time with the same tracker and room. It checks
          STUN, tracker announce, and direct UDP hole-punch reachability before we depend on it for
          terminal traffic.
        </p>
        <pre v-if="probeLog" class="probe-log">{{ probeLog }}</pre>
        <p v-if="error" class="error">{{ error }}</p>
      </section>

      <section class="card" v-else>
        <h2>Starting host service</h2>
        <p>{{ status }}</p>
      </section>
    </main>

    <main v-else-if="screen === 'connect'" class="connect-layout">
      <div class="focus-icon">▦</div>
      <p class="helper">
        Scan a QR code from the host app to establish a secure terminal connection.
      </p>
      <button class="primary" @click="openScanner">Scan QR Code</button>
      <textarea v-model="pairingText" placeholder="Or paste pairing JSON here" maxlength="4096" />
      <input
        v-model="pairingPassword"
        type="password"
        placeholder="Password, if host requires one"
        autocomplete="current-password"
        maxlength="256"
      />
      <button class="secondary" @click="startTerminal">Connect</button>
      <button class="secondary" :disabled="probeRunning" @click="runInternetP2pTest">
        {{ probeRunning ? "Testing…" : "Test internet P2P first" }}
      </button>
      <pre v-if="probeLog" class="probe-log">{{ probeLog }}</pre>
      <p v-if="error" class="error">{{ error }}</p>
    </main>

    <main v-else-if="screen === 'scanner'" class="scanner-layout">
      <video ref="videoEl" class="camera-feed" muted playsinline />
      <button
        class="back"
        @click="
          stopScanner();
          screen = 'connect';
        "
      >
        ←
      </button>
      <div class="scanner-box">
        <span />
      </div>
      <div class="scanner-help">{{ scannerMessage }}</div>
      <textarea v-model="pairingText" placeholder="Paste pairing JSON" maxlength="4096" />
      <input
        v-model="pairingPassword"
        type="password"
        placeholder="Password, if required"
        autocomplete="current-password"
        maxlength="256"
      />
      <button class="primary" @click="startTerminal">Connect</button>
    </main>

    <main v-else class="terminal-layout">
      <div ref="terminalEl" class="terminal-output" @click="focusTerminal" />
      <nav class="bottom-actions">
        <button aria-label="Ctrl-C" title="Ctrl-C" @click="sendRaw('\u0003')">⌃C</button>
        <button aria-label="Tab" title="Tab" @click="sendRaw('\t')">⇥</button>
        <button aria-label="Keyboard" title="Keyboard" @click="focusTerminal">⌨</button>
        <button aria-label="Copy selection" title="Copy selection" @click="copyTerminalSelection">
          ⧉
        </button>
        <button class="danger" aria-label="Disconnect" title="Disconnect" @click="disconnect">
          <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
            <path d="M12 3v10" />
            <path d="M6.7 6.7a8 8 0 1 0 10.6 0" />
          </svg>
        </button>
      </nav>
    </main>
  </div>
</template>

<style>
:root {
  --keyboard-inset: 0px;
  font-family: "Roboto Flex", Inter, system-ui, sans-serif;
  color: #e5e2e1;
  background: #131313;
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
}
* {
  box-sizing: border-box;
}
html,
body,
#app {
  min-width: 320px;
  min-height: 100%;
  height: 100%;
  background: #131313;
  overflow: hidden;
}
body {
  margin: 0;
}
button,
input,
textarea {
  font: inherit;
}
.app-shell {
  height: 100vh;
  height: 100dvh;
  display: flex;
  flex-direction: column;
  background: #131313;
  padding-top: env(safe-area-inset-top);
  overflow: hidden;
}
.topbar {
  min-height: 64px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 8px 16px;
  background: #131313;
  border-bottom: 1px solid #43474c;
}
.topbar h1 {
  margin: 0;
  font-size: 22px;
  line-height: 28px;
  font-weight: 500;
}
.topbar p {
  margin: 0;
  color: #b9c8de;
  font-size: 14px;
  display: flex;
  gap: 6px;
  align-items: center;
}
.icon-button {
  width: 48px;
  height: 48px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: #c4c6cd;
  font-size: 22px;
}
.icon-button:active,
button:active {
  transform: scale(0.97);
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  display: inline-block;
  background: #43474c;
}
.dot.ok {
  background: #10b981;
}
.host-layout,
.connect-layout {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 24px;
  padding: 24px;
}
.card {
  width: min(920px, 100%);
  background: #201f1f;
  border: 1px solid #43474c;
  border-radius: 24px;
  padding: 24px;
}
.intro {
  background: #1c1b1b;
}
.card h2 {
  margin: 0 0 8px;
  font-size: 28px;
  line-height: 36px;
}
.card p,
.helper {
  color: #c4c6cd;
  line-height: 24px;
}
.qr-card {
  display: grid;
  grid-template-columns: minmax(220px, 300px) 1fr;
  gap: 24px;
  align-items: center;
}
.qr {
  background: #fff;
  border-radius: 16px;
  padding: 16px;
  display: grid;
  place-items: center;
}
.qr svg {
  width: 100%;
  height: auto;
  display: block;
}
dl {
  display: grid;
  gap: 10px;
  margin: 20px 0;
}
dt {
  color: #8e9197;
  font-size: 12px;
  text-transform: uppercase;
  letter-spacing: 0.08em;
}
dd {
  margin: 0;
  font-family: "JetBrains Mono", ui-monospace, monospace;
  overflow-wrap: anywhere;
}
.settings-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(160px, 240px) auto;
  gap: 12px;
  align-items: end;
  margin-top: 16px;
}
.settings-grid label {
  display: grid;
  gap: 6px;
  color: #c4c6cd;
  font-size: 14px;
}
.settings-grid input,
.settings-grid select {
  min-height: 48px;
  border-radius: 12px;
  border: 1px solid #43474c;
  background: #1c1b1b;
  color: #e5e2e1;
  padding: 0 12px;
  font-family: "JetBrains Mono", ui-monospace, monospace;
}
.primary,
.secondary,
.danger {
  min-height: 48px;
  border-radius: 999px;
  padding: 0 24px;
  border: 1px solid transparent;
  cursor: pointer;
}
.primary {
  background: #d0e4ff;
  color: #1e3246;
}
.secondary {
  background: transparent;
  color: #d0e4ff;
  border-color: #8e9197;
}
.danger {
  color: #ffb4ab;
  background: transparent;
}
.focus-icon {
  width: 128px;
  height: 128px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  background: #201f1f;
  border: 1px solid #43474c;
  font-size: 64px;
  color: #fff;
}
.connect-layout textarea,
.scanner-layout textarea {
  width: min(560px, 100%);
  min-height: 120px;
  border-radius: 16px;
  border: 1px solid #43474c;
  background: #1c1b1b;
  color: #e5e2e1;
  padding: 14px;
  font-family: "JetBrains Mono", ui-monospace, monospace;
  z-index: 2;
}
.connect-layout input,
.scanner-layout input {
  width: min(560px, 100%);
  min-height: 48px;
  border-radius: 16px;
  border: 1px solid #43474c;
  background: #1c1b1b;
  color: #e5e2e1;
  padding: 0 14px;
  z-index: 2;
}
.error {
  color: #ffb4ab;
}
.scanner-layout {
  flex: 1;
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 20px;
  padding: 24px;
  background: #0e0e0e;
  overflow: hidden;
}
.camera-feed {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
  opacity: 0.62;
  filter: saturate(0.75);
}
.back {
  position: absolute;
  top: calc(16px + env(safe-area-inset-top));
  left: 16px;
  width: 48px;
  height: 48px;
  border-radius: 999px;
  border: 1px solid #43474c;
  background: #2a2a2a;
  color: #fff;
}
.scanner-box {
  width: 260px;
  height: 260px;
  border-radius: 16px;
  border: 3px solid #fff;
  box-shadow: 0 0 0 9999px rgba(19, 19, 19, 0.7);
  position: relative;
}
.scanner-box span {
  position: absolute;
  left: 24px;
  right: 24px;
  top: 50%;
  height: 1px;
  background: rgba(255, 255, 255, 0.4);
}
.scanner-help {
  max-width: 360px;
  text-align: center;
  background: rgba(42, 42, 42, 0.9);
  border: 1px solid #43474c;
  border-radius: 999px;
  padding: 10px 16px;
}
.terminal-layout {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: #0c0c0c;
  padding-bottom: var(--keyboard-inset);
  transition: padding-bottom 120ms ease-out;
}
.probe-log {
  width: 100%;
  max-height: 220px;
  overflow: auto;
  margin: 12px 0 0;
  padding: 12px;
  border-radius: 12px;
  background: #0e0e0e;
  color: #a3be8c;
  font:
    12px/18px "JetBrains Mono",
    ui-monospace,
    monospace;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.terminal-output {
  flex: 1;
  min-height: 0;
  width: 100%;
  overflow: hidden;
  background: #0c0c0c;
  padding: 6px;
}
.terminal-output .xterm {
  height: 100%;
}
.terminal-output .xterm-viewport {
  background: #0c0c0c !important;
}
.terminal-output .xterm-screen {
  touch-action: manipulation;
}
.bottom-actions {
  flex: 0 0 auto;
  min-height: calc(56px + env(safe-area-inset-bottom));
  display: flex;
  align-items: center;
  justify-content: space-around;
  gap: 4px;
  padding: 6px 8px calc(6px + env(safe-area-inset-bottom));
  background: #201f1f;
  border-top: 1px solid #43474c;
}
.bottom-actions button {
  min-width: 52px;
  min-height: 44px;
  border: 0;
  border-radius: 12px;
  background: transparent;
  color: #c4c6cd;
  font-size: 22px;
}
.bottom-actions button svg {
  width: 24px;
  height: 24px;
  display: block;
  margin: auto;
  fill: none;
  stroke: currentColor;
  stroke-width: 2;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.bottom-actions .danger {
  color: #ffb4ab;
}
@media (max-width: 720px) {
  .qr-card,
  .settings-grid {
    grid-template-columns: 1fr;
  }
  .host-layout,
  .connect-layout {
    padding: 16px;
    justify-content: flex-start;
    padding-top: calc(24px + env(safe-area-inset-top));
    padding-bottom: calc(24px + env(safe-area-inset-bottom));
  }
}
</style>

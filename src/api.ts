import { Channel, invoke } from "@tauri-apps/api/core";
import type { HostServiceInfo, PairingPayload, RuntimeInfo, TerminalEvent, TerminalOption, TerminalShell } from "./types";

export function runtimeInfo() {
  return invoke<RuntimeInfo>("runtime_info");
}

export function serviceInfo() {
  return invoke<HostServiceInfo>("service_info");
}

export function pairingPayload() {
  return invoke<PairingPayload>("pairing_payload");
}

export function availableTerminals() {
  return invoke<TerminalOption[]>("available_terminals");
}

export function updateHostSettings(shell: TerminalShell, password?: string) {
  return invoke<HostServiceInfo>("update_host_settings", { shell, password });
}

export function enableTrackerPairing(tracker: string, trackerRoom?: string) {
  return invoke<HostServiceInfo>("enable_tracker_pairing", { tracker, trackerRoom });
}

export function enableRelayPairing(relay: string, relayCertSha256?: string) {
  return invoke<HostServiceInfo>("enable_relay_pairing", { relay, relayCertSha256 });
}

export function connectTerminal(pairing: PairingPayload, onEvent: Channel<TerminalEvent>) {
  return invoke<string>("connect_terminal", { pairing, onEvent });
}

export function sendTerminalInput(sessionId: string, data: string) {
  return invoke<void>("send_terminal_input", { sessionId, data });
}

export function disconnectTerminal(sessionId: string) {
  return invoke<void>("disconnect_terminal", { sessionId });
}

export function testInternetP2p(tracker: string, room: string, onEvent: Channel<TerminalEvent>) {
  return invoke<void>("test_internet_p2p", { tracker, room, onEvent });
}

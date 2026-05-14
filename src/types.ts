export type TerminalShell = "cmd" | "powerShell" | "wsl";

export interface RuntimeInfo {
  platform: string;
  isDesktopHost: boolean;
}

export interface TerminalOption {
  shell: TerminalShell;
  label: string;
  available: boolean;
}

export interface PairingPayload {
  v: number;
  mode: "direct" | "tracker" | "relay";
  host?: string;
  token: string;
  certSha256: string;
  tracker?: string;
  trackerRoom?: string;
  relay?: string;
  relayCertSha256?: string;
  requiresPassword?: boolean;
  password?: string;
}

export interface HostServiceInfo {
  platform: string;
  running: boolean;
  status: string;
  listen: string;
  lanAddress?: string;
  certSha256: string;
  firstRun: boolean;
  shell: TerminalShell;
  passwordEnabled: boolean;
  pairing: PairingPayload;
  pairingJson: string;
  qrSvg: string;
}

export type TerminalEvent =
  | { event: "status"; data: string }
  | { event: "output"; data: string }
  | { event: "error"; data: string }
  | { event: "closed" };

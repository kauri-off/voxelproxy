// Typed bridge — the only file that touches window.__TAURI__

/* eslint-disable @typescript-eslint/no-explicit-any */
const { invoke } = (window as any).__TAURI__.core;
const { listen }  = (window as any).__TAURI__.event;

export interface LogEntry {
  level: 'info' | 'success' | 'warn' | 'error';
  message: string;
}

export interface ClientStatusPayload {
  which: 'primary' | 'secondary';
  online: boolean;
}

export interface UpdateInfo {
  tag: string;
  link: string;
}

// ── Commands ──────────────────────────────────────────────────────────────────

export async function startManualSession(serverAddr: string): Promise<void> {
  await invoke('start_manual_session', { serverAddr });
}

export async function startAutoSession(
  useWindivert: boolean,
  portMin: number,
  portMax: number,
): Promise<void> {
  await invoke('start_auto_session', { useWindivert, portMin, portMax });
}

export async function stopSession(): Promise<void> {
  await invoke('stop_session');
}

export async function getVersion(): Promise<string> {
  return await invoke('get_version');
}

export async function getLocalIpAddr(): Promise<string> {
  return await invoke('get_local_ip_addr');
}

export async function checkUpdates(): Promise<UpdateInfo | null> {
  return await invoke('check_updates');
}

export async function openUrl(url: string): Promise<void> {
  await invoke('open_url', { url });
}

// ── Events ────────────────────────────────────────────────────────────────────

type UnlistenFn = () => void;

export async function onProxyLog(cb: (e: LogEntry) => void): Promise<UnlistenFn> {
  return listen('proxy-log', (evt: { payload: LogEntry }) => cb(evt.payload));
}

export async function onSessionStarted(cb: () => void): Promise<UnlistenFn> {
  return listen('session-started', () => cb());
}

export async function onSessionEnded(cb: () => void): Promise<UnlistenFn> {
  return listen('session-ended', () => cb());
}

export async function onClientStatus(
  cb: (e: ClientStatusPayload) => void,
): Promise<UnlistenFn> {
  return listen('client-status', (evt: { payload: ClientStatusPayload }) => cb(evt.payload));
}

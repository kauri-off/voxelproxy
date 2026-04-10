import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { LogEntry } from './types';

export interface ClientStatusPayload {
  which: 'primary' | 'secondary';
  online: boolean;
}

export interface UpdateInfo {
  tag: string;
  link: string;
}

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

export async function getPlatform(): Promise<string> {
  return await invoke('get_platform');
}

export async function setPanicMode(value: boolean): Promise<void> {
  await invoke('set_panic_mode', { value });
}

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

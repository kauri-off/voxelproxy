export type Phase = 'idle' | 'waiting' | 'active';
export type Mode = 'manual' | 'auto';

export interface ClientStatus {
  online: boolean;
}

export interface AppState {
  phase: Phase;
  mode: Mode;
  manualServerAddr: string;
  autoUseWindivert: boolean;
  autoPortMin: number;
  autoPortMax: number;
  localIp: string;
  version: string;
  updateInfo: { tag: string; link: string } | null;
  clients: {
    primary: ClientStatus;
    secondary: ClientStatus;
  };
}

export interface LogEntry {
  level: 'info' | 'warn' | 'error' | 'success';
  message: string;
}
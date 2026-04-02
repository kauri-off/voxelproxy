import type { UpdateInfo } from './tauri.js';

export type SessionPhase = 'idle' | 'waiting' | 'active';
export type AppMode = 'manual' | 'auto';

export interface ClientState {
  online: boolean;
}

export interface AppState {
  phase: SessionPhase;
  mode: AppMode;
  manualServerAddr: string;
  autoUseWindivert: boolean;
  autoPortMin: number;
  autoPortMax: number;
  localIp: string;
  version: string;
  updateInfo: UpdateInfo | null;
  clients: {
    primary: ClientState;
    secondary: ClientState;
  };
}

const DEFAULT_STATE: AppState = {
  phase: 'idle',
  mode: 'manual',
  manualServerAddr: '',
  autoUseWindivert: true,
  autoPortMin: 25560,
  autoPortMax: 25570,
  localIp: '…',
  version: '',
  updateInfo: null,
  clients: {
    primary: { online: false },
    secondary: { online: false },
  },
};

type Listener = (state: AppState) => void;

export class AppStore {
  private state: AppState = { ...DEFAULT_STATE, clients: { primary: { online: false }, secondary: { online: false } } };
  private listeners: Listener[] = [];

  getState(): AppState {
    return this.state;
  }

  setState(patch: Partial<AppState>): void {
    this.state = { ...this.state, ...patch };
    for (const fn of this.listeners) fn(this.state);
  }

  subscribe(listener: Listener): () => void {
    this.listeners.push(listener);
    return () => {
      this.listeners = this.listeners.filter(l => l !== listener);
    };
  }
}

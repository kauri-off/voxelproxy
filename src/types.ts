export type Phase = 'idle' | 'running';
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
  updateProcessed: boolean;
  updateError: boolean;
  updateDownloading: boolean;
  updateProgress: number | null;
  updateInstallError: string | null;
  clients: {
    primary: ClientStatus;
    secondary: ClientStatus;
  };
  platform: string;
  panicMode: boolean,
  nickName: string,
  serverAddr: string,
}
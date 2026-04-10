import { useState, useEffect, useCallback } from "react";
import * as api from "../tauri";
import { AppState, LogEntry } from "../types";

const initialState: AppState = {
  phase: "idle",
  mode: "manual",
  manualServerAddr: "",
  autoUseWindivert: true,
  autoPortMin: 25560,
  autoPortMax: 25570,
  localIp: "...",
  version: "",
  updateInfo: null,
  updateProcessed: false,
  updateError: false,
  clients: { primary: { online: false }, secondary: { online: false } },
  platform: '',
  panicMode: false
};

export function useAppState() {
  const [state, setState] = useState<AppState>(initialState);
  const [logs, setLogs] = useState<LogEntry[]>([]);

  const addLog = useCallback((level: LogEntry["level"], message: string) => {
    setLogs((prev) => [...prev, { level, message }]);
  }, []);

  useEffect(() => {
    const loadVersion = async () => {
      try {
        const version = await api.getVersion();
        setState((prev) => ({ ...prev, version }));
      } catch (err) {
        addLog("error", `Ошибка загрузки версии: ${err}`);
      }
    };

    const loadLocalIp = async () => {
      try {
        const localIp = await api.getLocalIpAddr();
        setState((prev) => ({ ...prev, localIp }));
      } catch (err) {
        addLog("error", `Ошибка загрузки IP-адреса: ${err}`);
      }
    };

    const loadUpdateInfo = async () => {
      try {
        const updateInfo = await api.checkUpdates();
        setState((prev) => ({
          ...prev,
          updateInfo,
          updateProcessed: true,
          updateError: false,
        }));
      } catch (err) {
        addLog("error", `Ошибка проверки обновлений: ${err}`);
        setState((prev) => ({
          ...prev,
          updateInfo: null,
          updateProcessed: true,
          updateError: true,
        }));
      }
    };

    const loadPlatform = async () => {
      try {
        const platform = await api.getPlatform();
        setState((prev) => ({
          ...prev,
          platform,
          autoUseWindivert: platform === 'windows' ? prev.autoUseWindivert : false,
        }));
      } catch (err) {
        addLog("error", `Ошибка получения платформы: ${err}`);
      }
    };

    loadVersion();
    loadLocalIp();
    loadUpdateInfo();
    loadPlatform();
  }, [addLog]);

  return { state, setState, logs, setLogs, addLog };
}
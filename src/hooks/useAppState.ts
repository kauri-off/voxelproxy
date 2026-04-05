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
  clients: { primary: { online: false }, secondary: { online: false } },
};

export function useAppState() {
  const [state, setState] = useState<AppState>(initialState);
  const [logs, setLogs] = useState<LogEntry[]>([]);

  const addLog = useCallback((level: LogEntry["level"], message: string) => {
    setLogs((prev) => [...prev, { level, message }]);
  }, []);

  useEffect(() => {
    const load = async () => {
      try {
        const [version, ip, updateInfo] = await Promise.all([
          api.getVersion(),
          api.getLocalIpAddr(),
          api.checkUpdates(),
        ]);

        setState((s) => ({
          ...s,
          version,
          localIp: ip,
          updateInfo,
        }));
      } catch (err) {
        addLog("error", `Ошибка инициализации: ${err}`);
      }
    };

    load();
  }, [addLog]);

  return { state, setState, logs, setLogs, addLog };
}
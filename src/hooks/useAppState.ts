import { useState, useEffect, useCallback } from "react";
import { commands, events, LogLevel, ProxyLogEvent } from "../bindings";
import { AppState } from "../types";

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
  updateDownloading: false,
  updateProgress: null,
  updateInstallError: null,
  clients: { primary: { online: false }, secondary: { online: false } },
  platform: '',
  panicMode: false,
  nickName: '',
  serverAddr: '',
};

export function useAppState() {
  const [state, setState] = useState<AppState>(initialState);
  const [logs, setLogs] = useState<ProxyLogEvent[]>([]);

  const addLog = useCallback((level: LogLevel, message: string) => {
    setLogs((prev) => [...prev, { level, message }]);
  }, []);

  useEffect(() => {
    const loadVersion = async () => {
      try {
        const version = await commands.getVersion();
        setState((prev) => ({ ...prev, version }));
      } catch (err) {
        addLog("Error", `Ошибка загрузки версии: ${err}`);
      }
    };

    const loadLocalIp = async () => {
      try {
        const localIp = await commands.getLocalIpAddr();
        setState((prev) => ({ ...prev, localIp }));
      } catch (err) {
        addLog("Error", `Ошибка загрузки IP-адреса: ${err}`);
      }
    };

    const loadUpdateInfo = async () => {
      const result = await commands.checkUpdates();

      const isError = result.status === "error";

      if (isError) {
        addLog("Error", `Ошибка проверки обновлений: ${result.error}`);
      }

      setState((prev) => ({
        ...prev,
        updateInfo: isError ? null : result.data,
        updateProcessed: true,
        updateError: isError,
      }));
    };

    const loadPlatform = async () => {
      try {
        const platform = await commands.getPlatform();
        setState((prev) => ({
          ...prev,
          platform,
          autoUseWindivert: platform === 'windows' ? prev.autoUseWindivert : false,
        }));
      } catch (err) {
        addLog("Error", `Ошибка получения платформы: ${err}`);
      }
    };

    loadVersion();
    loadLocalIp();
    loadUpdateInfo();
    loadPlatform();

    const unlistenPromise = events.updateProgressEvent.listen(({ payload }) => {
      const { downloaded, total } = payload;
      const pct =
        total > 0
          ? Math.min(100, Math.floor((Number(downloaded) * 100) / Number(total)))
          : null;
      setState((prev) => ({ ...prev, updateProgress: pct }));
    });
    return () => {
      unlistenPromise.then((fn) => fn()).catch(() => {});
    };
  }, [addLog]);

  return { state, setState, logs, setLogs, addLog };
}
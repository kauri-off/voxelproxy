import { useEffect } from "react";
import * as api from "../tauri";
import { AppState, LogEntry } from "../types";

export function useTauriListeners(
  setState: React.Dispatch<React.SetStateAction<AppState>>,
  addLog: (level: LogEntry["level"], message: string) => void
) {
  useEffect(() => {
    let unlisteners: Array<() => void> = [];

    const setup = async () => {
      const unlog = await api.onProxyLog((log) =>
        addLog(log.level, log.message)
      );

      const unstart = await api.onSessionStarted(() => {
        setState((s) => ({ ...s,
          phase: "running",
          clients: { primary: { online: false }, secondary: { online: false } },
        }));
        addLog("info", "Сессия запущена, ожидание клиентов...");
      });

      const unend = await api.onSessionEnded(() => {
        setState((s) => ({ ...s, phase: "idle" }));
        addLog("info", "Сессия остановлена");
      });

      const unclient = await api.onClientStatus(({ which, online }) => {
        setState((prev) => {
          const clients = {
            ...prev.clients,
            [which]: { online },
          };

          return { ...prev, clients };
        });

        addLog(
          "info",
          `${which === "primary" ? "Основное" : "Второе"} устройство ${
            online ? "подключилось" : "отключилось"
          }`
        );
      });

      const unnickname = await api.onNickName((nickname) => {
        setState((s) => ({...s, nickName: nickname}))
      })

      unlisteners = [unlog, unstart, unend, unclient, unnickname];
    };

    setup();

    return () => {
      unlisteners.forEach((fn) => fn());
    };
  }, [setState, addLog]);
}
import { useEffect } from "react";
import { events, LogLevel } from "../bindings";
import { AppState } from "../types";

export function useTauriListeners(
  setState: React.Dispatch<React.SetStateAction<AppState>>,
  addLog: (level: LogLevel, message: string) => void
) {
  useEffect(() => {
    let unlisteners: Array<() => void> = [];

    const setup = async () => {
      const unlog = await events.proxyLogEvent.listen((e) => addLog(e.payload.level, e.payload.message));

      const unstart = await events.sessionStartedEvent.listen(() => {
        setState((s) => ({ ...s,
          phase: "running",
          clients: { primary: { online: false }, secondary: { online: false } },
        }));
      });

      const unend = await events.sessionEndedEvent.listen(() => {
        setState((s) => ({ ...s, phase: "idle" }));
      });

      const unclient = await events.clientStatusEvent.listen((e) => {
        const { which, online } = e.payload;
        setState((prev) => {
          const key = which.toLowerCase() as "primary" | "secondary";
          const clients = {
            ...prev.clients,
            [key]: { online },
          };

          return { ...prev, clients };
        });
      });

      const unnickname = await events.nickNameEvent.listen((e) => {
        setState((s) => ({...s, nickName: e.payload}))
      })

      unlisteners = [unlog, unstart, unend, unclient, unnickname];
    };

    setup();

    return () => {
      unlisteners.forEach((fn) => fn());
    };
  }, [setState, addLog]);
}
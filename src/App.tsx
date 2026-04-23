import { useCallback, useEffect, useState } from "react";
import { useAppState } from "./hooks/useAppState";
import { useTauriListeners } from "./hooks/useTauriListeners";
import { TitleBar } from "./components/TitleBar";
import { LogPanel } from "./components/LogPanel";
import { ChangelogModal } from "./components/ChangelogModal";
import { IdleView } from "./views/IdleView";
import { RunningView } from "./views/RunningView";
import * as api from "./bindings";
import { ChangelogEntry } from "./bindings";

export const App = () => {
  const { state, setState, logs, setLogs, addLog } = useAppState();
  useTauriListeners(setState, addLog);

  const [changelog, setChangelog] = useState<ChangelogEntry[]>([]);

  useEffect(() => {
    (async () => {
      const result = await api.commands.getPendingChangelogs();
      if (result.status === "ok" && result.data.length > 0) {
        setChangelog(result.data);
      }
    })();
  }, []);

  const dismissChangelog = useCallback(() => {
    setChangelog([]);
    api.commands.acknowledgeChangelog();
  }, []);

  const handleStop = useCallback(async () => {
    const result = await api.commands.stopSession();
    if (result.status === "error") {
      addLog("Error", result.error);
    }
  }, [addLog]);

  const togglePanicMode = () => {
    setState((prev) => {
      const newMode = !prev.panicMode;
      api.commands.setPanicMode(newMode);
      return { ...prev, panicMode: newMode };
    });
  };

  const isIdle = state.phase === "idle";

  return (
    <div className="app">
      <TitleBar state={state} onStop={handleStop} />

      <main className="view-container">
        <div className={`view ${isIdle ? "is-visible" : ""}`}>
          <IdleView state={state} setState={setState} addLog={addLog} />
        </div>

        <div className={`view ${!isIdle ? "is-visible" : ""}`}>
          <RunningView state={state} onTogglePanicMode={togglePanicMode} />
        </div>
      </main>

      <LogPanel logs={logs} onClear={() => setLogs([])} />

      {changelog.length > 0 && (
        <ChangelogModal entries={changelog} onDismiss={dismissChangelog} />
      )}
    </div>
  );
};

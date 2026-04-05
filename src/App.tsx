import { useCallback } from "react";
import { useAppState } from "./hooks/useAppState";
import { useTauriListeners } from "./hooks/useTauriListeners";
import { TitleBar } from "./components/TitleBar";
import { LogPanel } from "./components/LogPanel";
import { IdleView } from "./views/IdleView";
import { RunningView } from "./views/RunningView";
import * as api from "./tauri";

export const App = () => {
  const { state, setState, logs, setLogs, addLog } = useAppState();
  useTauriListeners(setState, addLog);

  const handleStop = useCallback(() => {
    api.stopSession().catch((err) => {
      addLog("error", `Не удалось остановить: ${err}`);
    });
  }, [addLog]);

  const isIdle = state.phase === "idle";

  return (
    <div className="app">
      <TitleBar state={state} onStop={handleStop} />

      <main className="view-container">
        <div className={`view ${isIdle ? "is-visible" : ""}`}>
          <IdleView state={state} setState={setState} addLog={addLog} />
        </div>

        <div className={`view ${!isIdle ? "is-visible" : ""}`}>
          <RunningView state={state} />
        </div>
      </main>

      <LogPanel logs={logs} onClear={() => setLogs([])} />
    </div>
  );
};

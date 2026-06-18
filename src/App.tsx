import { useCallback, useEffect, useState } from "react";
import { AnimatePresence, MotionConfig, motion } from "motion/react";
import { useAppState } from "./hooks/useAppState";
import { useTauriListeners } from "./hooks/useTauriListeners";
import { TitleBar } from "./components/TitleBar";
import { LogPanel } from "./components/LogPanel";
import { ChangelogModal } from "./components/ChangelogModal";
import { DeveloperMessageModal } from "./components/DeveloperMessageModal";
import { IdleView } from "./views/IdleView";
import { RunningView } from "./views/RunningView";
import * as api from "./bindings";
import { ChangelogEntry } from "./bindings";

export const App = () => {
  const { state, setState, logs, setLogs, addLog } = useAppState();
  useTauriListeners(setState, addLog);

  const [changelog, setChangelog] = useState<ChangelogEntry[]>([]);
  const [showDeveloperMessage, setShowDeveloperMessage] = useState(false);

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
    <MotionConfig reducedMotion="user">
      <div className="app">
        <TitleBar
          state={state}
          onStop={handleStop}
          onContact={() => setShowDeveloperMessage(true)}
        />

        <main className="view-container">
          <AnimatePresence mode="wait" initial={false}>
            <motion.div
              key={isIdle ? "idle" : "running"}
              className="view"
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              transition={{ duration: 0.22, ease: "easeInOut" }}
            >
              {isIdle ? (
                <IdleView state={state} setState={setState} addLog={addLog} />
              ) : (
                <RunningView state={state} onTogglePanicMode={togglePanicMode} />
              )}
            </motion.div>
          </AnimatePresence>
        </main>

        <LogPanel logs={logs} onClear={() => setLogs([])} />

        {changelog.length > 0 && (
          <ChangelogModal entries={changelog} onDismiss={dismissChangelog} />
        )}

        {showDeveloperMessage && (
          <DeveloperMessageModal
            onClose={() => setShowDeveloperMessage(false)}
            addLog={addLog}
          />
        )}
      </div>
    </MotionConfig>
  );
};

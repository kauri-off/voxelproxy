import React, { useState, useCallback } from "react";
import { AppState } from "../types";
import { commands, LogLevel } from "../bindings";

const validateManualAddr = (addr: string): boolean => {
  const trimmed = addr.trim();
  if (!trimmed) return false;

  const lastColon = trimmed.lastIndexOf(":");
  let host: string;

  if (lastColon !== -1 && lastColon !== 0) {
    host = trimmed.substring(0, lastColon);
    const portStr = trimmed.substring(lastColon + 1);
    if (!/^\d+$/.test(portStr)) return false;
    const p = parseInt(portStr, 10);
    if (p < 1 || p > 65535) return false;
  } else {
    host = trimmed;
  }

  const isIPv4 = /^(\d{1,3}\.){3}\d{1,3}$/.test(host);
  if (isIPv4) {
    const parts = host.split(".");
    return parts.every((part) => {
      const num = parseInt(part, 10);
      return num >= 0 && num <= 255;
    });
  }

  const domainRegex = /^(?!-)[A-Za-z0-9-]{1,63}(?<!-)(\.[A-Za-z0-9-]{1,63})*$/;
  return domainRegex.test(host);
};

interface Props {
  state: AppState;
  setState: React.Dispatch<React.SetStateAction<AppState>>;
  addLog: (level: LogLevel, message: string) => void;
}

export const IdleView: React.FC<Props> = ({ state, setState, addLog }) => {
  const [isStarting, setIsStarting] = useState(false);

  const start = useCallback(async () => {
    if (isStarting) return;
    setIsStarting(true);

    try {
      if (state.mode === "manual") {
        if (!state.manualServerAddr.trim()) {
          addLog("Error", "Введите адрес сервера");
          return;
        }
        if (!validateManualAddr(state.manualServerAddr)) {
          addLog(
            "Error",
            "Неверный формат адреса (пример: mc.funtime.su:25565)",
          );
          return;
        }

        const result = await commands.startManualSession(
          state.manualServerAddr,
        );

        if (result.status === "error") {
          addLog("Error", `Ошибка запуска: ${result.error}`);
        }
      } else {
        const { autoUseWindivert, autoPortMin, autoPortMax } = state;

        if (
          autoPortMin < 1 ||
          autoPortMax > 65535 ||
          autoPortMin > autoPortMax
        ) {
          addLog("Error", "Неверный диапазон портов (1-65535, min ≤ max)");
          return;
        }

        setState((prev) => ({ ...prev, panicMode: false }));
        const resultpanic = await commands.setPanicMode(false);
        if (resultpanic.status === "error") {
          addLog(
            "Error",
            `Ошибка установки режима проверки: ${resultpanic.error}`,
          );
        }

        const result = await commands.startAutoSession(
          autoUseWindivert,
          autoPortMin,
          autoPortMax,
        );

        if (result.status === "error") {
          addLog("Error", `Ошибка запуска: ${result.error}`);
        }
      }
    } finally {
      setIsStarting(false);
    }
  }, [state, isStarting, addLog]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      start();
    }
  };

  const hasUpdate = state.updateInfo !== null;
  const isError = state.updateError;
  const isReady = state.updateProcessed && !hasUpdate && !isError;
  const isPending = !state.updateProcessed;

  const isBlocked = isPending || hasUpdate || isError || isStarting;

  let buttonText = "";
  let buttonAction = () => {};
  let isButtonDisabled = false;

  if (hasUpdate) {
    buttonText = `Обновиться (${state.updateInfo!.tag})`;
    buttonAction = () => commands.openUrl(state.updateInfo!.link);
    isButtonDisabled = false;
  } else if (isError) {
    buttonText = "Ошибка проверки обновления";
    buttonAction = () =>
      commands.openUrl("https://github.com/kauri-off/voxelproxy/releases");
    isButtonDisabled = false;
  } else if (isPending) {
    buttonText = "...";
    buttonAction = () => {};
    isButtonDisabled = true;
  } else if (isReady) {
    buttonText = isStarting ? "Запуск..." : "Запустить";
    buttonAction = start;
    isButtonDisabled = isStarting;
  }

  return (
    <div className="idle-view__body">
      <div className="config-card">
        <div className="config-card__section">
          <span className="section-label">Режим работы</span>
          <div className="mode-tabs">
            <button
              className={`tab ${state.mode === "manual" ? "active" : ""}`}
              onClick={() => setState((s) => ({ ...s, mode: "manual" }))}
              disabled={isBlocked}
            >
              Ручной
            </button>
            <button
              className={`tab ${state.mode === "auto" ? "active" : ""}`}
              onClick={() => setState((s) => ({ ...s, mode: "auto" }))}
              disabled={isBlocked}
            >
              Авто
            </button>
          </div>
        </div>

        <div className="config-card__section">
          {state.mode === "manual" ? (
            <>
              <span className="section-label">Целевой сервер</span>
              <input
                type="text"
                className="text-input"
                placeholder="mc.funtime.su"
                value={state.manualServerAddr}
                onChange={(e) =>
                  setState((s) => ({ ...s, manualServerAddr: e.target.value }))
                }
                onKeyDown={handleKeyDown}
                disabled={isBlocked}
              />
            </>
          ) : (
            <>
              <div className="windivert-row">
                <label
                  className={`checkbox-label ${state.platform !== "windows" ? "is-disabled" : ""}`}
                >
                  <input
                    type="checkbox"
                    checked={state.autoUseWindivert}
                    onChange={(e) =>
                      setState((s) => ({
                        ...s,
                        autoUseWindivert: e.target.checked,
                      }))
                    }
                    disabled={isBlocked || state.platform !== "windows"}
                  />
                  Использовать WinDivert (нужны права администратора)
                </label>
                {state.platform !== "windows" && (
                  <span
                    className="hint"
                    style={{ fontSize: "11px", color: "var(--c-muted)" }}
                  >
                    (только для Windows)
                  </span>
                )}
                <span
                  className="help-icon"
                  onClick={() =>
                    commands.openUrl(
                      "https://github.com/kauri-off/voxelproxy?tab=readme-ov-file#автоматический-режим-windows-хотспот",
                    )
                  }
                  role="button"
                  tabIndex={0}
                >
                  ?
                </span>
              </div>
              {state.autoUseWindivert && state.platform === "windows" && (
                <div className="port-range-row">
                  <span className="hint">Порты:</span>
                  <input
                    type="number"
                    className="port-input"
                    value={state.autoPortMin}
                    onChange={(e) =>
                      setState((s) => ({
                        ...s,
                        autoPortMin: Math.max(1, +e.target.value || 1),
                      }))
                    }
                    min={1}
                    max={65535}
                    disabled={isBlocked}
                  />
                  <span className="hint">–</span>
                  <input
                    type="number"
                    className="port-input"
                    value={state.autoPortMax}
                    onChange={(e) =>
                      setState((s) => ({
                        ...s,
                        autoPortMax: Math.min(65535, +e.target.value || 65535),
                      }))
                    }
                    min={1}
                    max={65535}
                    disabled={isBlocked}
                  />
                </div>
              )}
            </>
          )}
        </div>

        <button
          className="btn-primary"
          onClick={buttonAction}
          disabled={isButtonDisabled}
        >
          {buttonText}
        </button>
      </div>
    </div>
  );
};

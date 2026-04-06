import React, { useState, useCallback } from "react";
import { AppState } from "../types";
import * as api from "../tauri";

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
  addLog: (level: "info" | "error", message: string) => void;
}

export const IdleView: React.FC<Props> = ({ state, setState, addLog }) => {
  const [isStarting, setIsStarting] = useState(false);

  const start = useCallback(async () => {
    if (isStarting) return;
    setIsStarting(true);

    try {
      if (state.mode === "manual") {
        if (!state.manualServerAddr.trim()) {
          addLog("error", "Введите адрес сервера");
          return;
        }
        if (!validateManualAddr(state.manualServerAddr)) {
          addLog("error", "Неверный формат адреса (пример: mc.host:25565)");
          return;
        }
        await api.startManualSession(state.manualServerAddr);
        addLog("info", `Запуск ручной сессии к ${state.manualServerAddr}`);
      } else {
        const { autoUseWindivert, autoPortMin, autoPortMax } = state;
        if (
          autoPortMin < 1 ||
          autoPortMax > 65535 ||
          autoPortMin > autoPortMax
        ) {
          addLog("error", "Неверный диапазон портов (1-65535, min ≤ max)");
          return;
        }
        await api.startAutoSession(autoUseWindivert, autoPortMin, autoPortMax);
        addLog(
          "info",
          `Запуск авто-сессии (WinDivert: ${autoUseWindivert}, порты: ${autoPortMin}-${autoPortMax})`,
        );
      }
    } catch (err: any) {
      addLog("error", `Ошибка запуска: ${err.message || err}`);
    } finally {
      setIsStarting(false);
    }
  }, [state, isStarting, addLog]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      start();
    }
  };

  const handleUpdateClick = () => {
    if (state.updateInfo) {
      api.openUrl(state.updateInfo.link);
    }
  };

  const hasUpdate = state.updateInfo !== null;

  return (
    <div className="idle-view__body">
      <div className="config-card">
        <div className="config-card__section">
          <span className="section-label">Режим работы</span>
          <div className="mode-tabs">
            <button
              className={`tab ${state.mode === "manual" ? "active" : ""}`}
              onClick={() => setState((s) => ({ ...s, mode: "manual" }))}
              disabled={isStarting || hasUpdate}
            >
              Ручной
            </button>
            <button
              className={`tab ${state.mode === "auto" ? "active" : ""}`}
              onClick={() => setState((s) => ({ ...s, mode: "auto" }))}
              disabled={isStarting || hasUpdate}
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
                disabled={isStarting || hasUpdate}
              />
            </>
          ) : (
            <>
              <div className="windivert-row">
                <label className="checkbox-label">
                  <input
                    type="checkbox"
                    checked={state.autoUseWindivert}
                    onChange={(e) =>
                      setState((s) => ({
                        ...s,
                        autoUseWindivert: e.target.checked,
                      }))
                    }
                    disabled={isStarting || hasUpdate}
                  />
                  Использовать WinDivert
                </label>
                <span
                  className="help-icon"
                  onClick={() =>
                    api.openUrl(
                      "https://github.com/kauri-off/voxelproxy?tab=readme-ov-file#автоматический-режим-windows-хотспот",
                    )
                  }
                  role="button"
                  tabIndex={0}
                >
                  ?
                </span>
              </div>
              {state.autoUseWindivert && (
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
                    disabled={isStarting || hasUpdate}
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
                    disabled={isStarting || hasUpdate}
                  />
                </div>
              )}
            </>
          )}
        </div>

        <button
          className="btn-primary"
          onClick={hasUpdate ? handleUpdateClick : start}
          disabled={hasUpdate ? false : isStarting}
        >
          {hasUpdate
            ? `Обновиться ${state.updateInfo?.tag ? `(${state.updateInfo.tag})` : ""}`
            : isStarting
              ? "Запуск..."
              : "Запустить"}
        </button>
      </div>
    </div>
  );
};

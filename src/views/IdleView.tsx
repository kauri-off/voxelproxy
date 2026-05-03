import React, { useState, useCallback, useEffect } from "react";
import { AppState } from "../types";
import { commands, LogLevel } from "../bindings";
import { ManualWarningModal } from "../components/ManualWarningModal";

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
  const [supportedVersions, setSupportedVersions] = useState<string[]>([]);
  const [showManualWarning, setShowManualWarning] = useState(false);

  useEffect(() => {
    commands.getSupportedVersions().then(setSupportedVersions);
  }, []);

  const launchManualSession = useCallback(async () => {
    const result = await commands.startManualSession(state.manualServerAddr);
    if (result.status === "error") {
      addLog("Error", `Ошибка запуска: ${result.error}`);
    }
  }, [state.manualServerAddr, addLog]);

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

        const acknowledged = await commands.getManualWarningAcknowledged();
        if (!acknowledged) {
          setShowManualWarning(true);
          return;
        }

        await launchManualSession();
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
  const isDownloading = state.updateDownloading;
  const installError = state.updateInstallError;

  const isBlocked =
    isPending || hasUpdate || isError || isStarting || isDownloading;

  const startUpdate = useCallback(async () => {
    if (!state.updateInfo) return;
    setState((s) => ({
      ...s,
      updateDownloading: true,
      updateProgress: null,
      updateInstallError: null,
    }));
    const r = await commands.downloadAndInstallUpdate(state.updateInfo.link);
    if (r.status === "error") {
      setState((s) => ({
        ...s,
        updateDownloading: false,
        updateInstallError: r.error,
      }));
      addLog("Error", `Ошибка установки обновления: ${r.error}`);
    }
  }, [state.updateInfo, setState, addLog]);

  let buttonText = "";
  let buttonAction = () => {};
  let isButtonDisabled = false;

  if (hasUpdate && installError) {
    buttonText = "Ошибка загрузки — повторить";
    buttonAction = () => {
      startUpdate();
    };
    isButtonDisabled = false;
  } else if (hasUpdate) {
    buttonText = `Обновиться (${state.updateInfo!.tag})`;
    buttonAction = () => {
      startUpdate();
    };
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

  const subtitle = isDownloading
    ? "Загрузка обновления…"
    : hasUpdate && installError
      ? "Ошибка загрузки обновления"
      : hasUpdate
        ? `Доступно обновление ${state.updateInfo!.tag}`
        : isError
          ? "Не удалось проверить обновления"
          : isPending
            ? "Проверка обновлений…"
            : "Готов к запуску";

  return (
    <div className="panel-host">
      <div className="panel">
        <div className="panel__header">
          <div className="panel__title">Настройка</div>
          <div className="panel__subtitle">{subtitle}</div>
        </div>

        <div className="field-row">
          <span className="field-row__label">Режим</span>
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

        {state.mode === "manual" ? (
          <div className="field-row">
            <span className="field-row__label">Сервер</span>
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
          </div>
        ) : (
          <>
            <div className="field-row">
              <span className="field-row__label">Хотспот</span>
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
                  Перехват трафика хотспота
                </label>
                {state.platform !== "windows" && (
                  <span className="hint">(только Windows)</span>
                )}
                <span
                  className="help-icon"
                  onClick={() =>
                    commands.openUrl(
                      "https://github.com/kauri-off/voxelproxy?tab=readme-ov-file#если-что-то-пошло-не-так",
                    )
                  }
                  role="button"
                  tabIndex={0}
                >
                  Инструкция
                </span>
              </div>
            </div>

            <div className="field-row">
              <span className="field-row__label" />
              <span className="hint auto-mode-note">
                Для двух клиентов через Wi-Fi-хотспот этого ПК. Запускайте
                от администратора.
              </span>
            </div>

            {state.autoUseWindivert && state.platform === "windows" && (
              <div className="field-row">
                <span className="field-row__label">Порты</span>
                <div className="port-range-row">
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
              </div>
            )}
          </>
        )}

        {supportedVersions.length > 0 && (
          <div className="field-row">
            <span className="field-row__label">Версии</span>
            <span className="idle-view__versions">
              {supportedVersions.join(", ")}
            </span>
          </div>
        )}

        {isDownloading ? (
          <div className="update-progress-block">
            <div className="update-progress-block__label">
              <span>Загрузка обновления</span>
              <span>
                {state.updateProgress !== null
                  ? `${state.updateProgress}%`
                  : "…"}
              </span>
            </div>
            <div
              className={`update-progress${
                state.updateProgress === null
                  ? " update-progress--indeterminate"
                  : ""
              }`}
            >
              <div
                className="update-progress__fill"
                style={
                  state.updateProgress !== null
                    ? { width: `${state.updateProgress}%` }
                    : undefined
                }
              />
            </div>
          </div>
        ) : (
          <button
            className="btn-primary idle-view__start"
            onClick={buttonAction}
            disabled={isButtonDisabled}
          >
            {buttonText}
          </button>
        )}
      </div>

      {showManualWarning && (
        <ManualWarningModal
          onCancel={() => setShowManualWarning(false)}
          onAcknowledge={async () => {
            setShowManualWarning(false);
            const ack = await commands.acknowledgeManualWarning();
            if (ack.status === "error") {
              addLog(
                "Warn",
                `Не удалось сохранить подтверждение: ${ack.error}`,
              );
            }
            setIsStarting(true);
            try {
              await launchManualSession();
            } finally {
              setIsStarting(false);
            }
          }}
        />
      )}
    </div>
  );
};

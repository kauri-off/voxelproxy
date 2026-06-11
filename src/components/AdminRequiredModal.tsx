import { useEffect, useState } from "react";
import { commands } from "../bindings";

interface Props {
  onCancel: () => void;
  onError: (message: string) => void;
}

export const AdminRequiredModal: React.FC<Props> = ({ onCancel, onError }) => {
  const [isRestarting, setIsRestarting] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  const restart = async () => {
    if (isRestarting) return;
    setIsRestarting(true);
    const result = await commands.relaunchAsAdmin();
    if (result.status === "error") {
      onError(result.error);
      setIsRestarting(false);
    }
    // On success the app exits and relaunches elevated, so nothing more to do.
  };

  return (
    <div
      className="changelog-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) onCancel();
      }}
    >
      <div
        className="changelog-card warning-card"
        role="alertdialog"
        aria-modal="true"
      >
        <div className="changelog-card__header warning-card__header">
          <span className="changelog-card__title">
            <span className="warning-card__icon" aria-hidden="true">
              ⚠
            </span>
            Нужны права администратора
          </span>
          <button
            className="changelog-card__close"
            onClick={onCancel}
            aria-label="Закрыть"
          >
            ✕
          </button>
        </div>

        <div className="changelog-card__body warning-card__body">
          <p>
            Авто-режим с чтением трафика раздачи Wi-Fi работает только{" "}
            <span className="warning-card__warn">от имени администратора</span>.
            Сейчас VoxelProxy запущен без повышенных прав.
          </p>
          <p>
            Нажмите кнопку ниже, чтобы перезапустить приложение с правами
            администратора (появится запрос UAC), затем снова нажмите
            «Запустить».
          </p>
        </div>

        <div className="changelog-card__footer warning-card__footer">
          <button className="warning-card__cancel" onClick={onCancel}>
            Отмена
          </button>
          <button
            className="warning-card__accept"
            onClick={restart}
            disabled={isRestarting}
          >
            {isRestarting
              ? "Перезапуск…"
              : "Перезапустить от администратора"}
          </button>
        </div>
      </div>
    </div>
  );
};

import { useEffect, useState } from "react";

interface Props {
  onAcknowledge: () => void;
  onCancel: () => void;
}

const COUNTDOWN_SECONDS = 10;

export const ManualWarningModal: React.FC<Props> = ({
  onAcknowledge,
  onCancel,
}) => {
  const [remaining, setRemaining] = useState(COUNTDOWN_SECONDS);

  useEffect(() => {
    if (remaining <= 0) return;
    const id = window.setTimeout(() => setRemaining((r) => r - 1), 1000);
    return () => window.clearTimeout(id);
  }, [remaining]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  const isReady = remaining <= 0;

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
            Внимание: <span className="warning-card__danger">опасный режим</span>
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
            Не используйте этот режим — он{" "}
            <span className="warning-card__danger">детектится</span>, и за его
            использование вас{" "}
            <span className="warning-card__danger">забанят</span> на сервере.
          </p>
          <p>
            Ручной режим оставлен только для{" "}
            <span className="warning-card__warn">отладки</span> и{" "}
            <span className="warning-card__warn">тестирования</span>. Для
            обычной игры используйте{" "}
            <span className="warning-card__safe">«Авто»</span> — он не
            детектится.
          </p>
          <p className="warning-card__confirm">
            Продолжая, вы подтверждаете, что{" "}
            <span className="warning-card__danger">понимаете риски</span> и
            берёте ответственность за возможный бан на себя.
          </p>
        </div>

        <div className="changelog-card__footer warning-card__footer">
          <button className="warning-card__cancel" onClick={onCancel}>
            Отмена
          </button>
          <button
            className="warning-card__accept"
            onClick={onAcknowledge}
            disabled={!isReady}
          >
            {isReady
              ? "Я понимаю риски, продолжить"
              : `Прочитайте предупреждение… ${remaining}`}
          </button>
        </div>
      </div>
    </div>
  );
};

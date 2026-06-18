import { useEffect, useState } from "react";
import { commands, LogLevel } from "../bindings";

interface Props {
  onClose: () => void;
  addLog: (level: LogLevel, message: string) => void;
}

export const DeveloperMessageModal: React.FC<Props> = ({ onClose, addLog }) => {
  const [message, setMessage] = useState("");
  const [isSending, setIsSending] = useState(false);
  const [isSent, setIsSent] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const canSend = message.trim().length > 0 && !isSending && !isSent;

  const send = async () => {
    if (!canSend) return;
    setIsSending(true);
    try {
      const result = await commands.sendDeveloperMessage(message.trim());
      if (result.status === "error") {
        addLog("Error", `Не удалось отправить сообщение: ${result.error}`);
        return;
      }
      setIsSent(true);
      window.setTimeout(onClose, 1500);
    } finally {
      setIsSending(false);
    }
  };

  return (
    <div
      className="changelog-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="changelog-card" role="dialog" aria-modal="true">
        <div className="changelog-card__header">
          <span className="changelog-card__title">Сообщение разработчику</span>
          <button
            className="changelog-card__close"
            onClick={onClose}
            aria-label="Закрыть"
          >
            ✕
          </button>
        </div>

        <div className="changelog-card__body">
          {isSent ? (
            <p className="dev-message__thanks">
              Спасибо! Сообщение отправлено.
            </p>
          ) : (
            <>
              <p className="dev-message__hint">
                Опишите проблему или предложение — текст уйдёт напрямую
                разработчику.
              </p>
              <textarea
                className="dev-message__textarea"
                placeholder="Опишите проблему или предложение…"
                value={message}
                onChange={(e) => setMessage(e.target.value)}
                autoFocus
                rows={6}
                disabled={isSending}
              />
            </>
          )}
        </div>

        {!isSent && (
          <div className="changelog-card__footer warning-card__footer">
            <button className="warning-card__cancel" onClick={onClose}>
              Отмена
            </button>
            <button
              className="changelog-card__ok"
              onClick={send}
              disabled={!canSend}
            >
              {isSending ? "Отправка…" : "Отправить"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
};

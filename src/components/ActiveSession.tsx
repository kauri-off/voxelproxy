import React, { useEffect, useState } from "react";
import { ClientStatus } from "../types";

interface Props {
  nickName: string;
  serverAddr: string;
  clients: {
    primary: ClientStatus;
    secondary: ClientStatus;
  };
  showPanicMode: boolean;
  panicMode: boolean;
  onTogglePanicMode: () => void;
}

function formatUptime(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const hours = Math.floor(s / 3600);
  const minutes = Math.floor((s % 3600) / 60);
  const seconds = s % 60;
  const pad = (n: number) => n.toString().padStart(2, "0");
  return hours > 0
    ? `${hours}:${pad(minutes)}:${pad(seconds)}`
    : `${pad(minutes)}:${pad(seconds)}`;
}

export const ActiveSession: React.FC<Props> = ({
  nickName,
  serverAddr,
  clients,
  showPanicMode,
  panicMode,
  onTogglePanicMode,
}) => {
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    const start = Date.now();
    const tick = () => setElapsed((Date.now() - start) / 1000);
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, []);

  return (
    <div className="panel">
      <div className="panel__header">
        <div className="active-session__nickname">
          Подключено: <span className="active-session__name">{nickName}</span>
        </div>

        {serverAddr && (
          <div className="active-session__server">
            Сервер: <span className="active-session__server-name">{serverAddr}</span>
          </div>
        )}
      </div>

      <div className="field-row">
        <span className="field-row__label">Время сессии</span>
        <span className="active-session__uptime">
          <span className="active-session__pulse" />
          {formatUptime(elapsed)}
        </span>
      </div>

      <div className="field-row">
        <span className="field-row__label">Клиенты</span>
        <span className="client-list">
          <span className="client-list__item">
            <span
              className={`client-dot ${clients.primary.online ? "client-dot--online" : "client-dot--offline"}`}
            />
            Основной
          </span>
          <span className="client-list__item">
            <span
              className={`client-dot ${clients.secondary.online ? "client-dot--online" : "client-dot--offline"}`}
            />
            Второй
          </span>
        </span>
      </div>

      {showPanicMode && (
        <button
          className={`panic-mode ${panicMode ? "panic-mode--on" : "panic-mode--off"}`}
          onClick={onTogglePanicMode}
        >
          Нажми когда будешь на проверке
        </button>
      )}
    </div>
  );
};

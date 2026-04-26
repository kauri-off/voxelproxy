import React, { useEffect, useState } from "react";
import { AppState } from "../types";
import { ActiveSession } from "../components/ActiveSession";
import { CheckIcon, CopyIcon } from "../components/Icons";

interface Props {
  state: AppState;
  onTogglePanicMode: () => void;
}

export const RunningView: React.FC<Props> = ({ state, onTogglePanicMode }) => {
  const anyOnline = state.clients.primary.online || state.clients.secondary.online;
  const bothOnline = state.clients.primary.online && state.clients.secondary.online;

  const [sessionLive, setSessionLive] = useState(false);
  useEffect(() => {
    if (bothOnline) setSessionLive(true);
    else if (!anyOnline) setSessionLive(false);
  }, [bothOnline, anyOnline]);

  const showPanicMode = state.mode === "auto";

  if (sessionLive) {
    return (
      <div className="panel-host">
        <ActiveSession
          nickName={state.nickName}
          serverAddr={state.serverAddr}
          clients={state.clients}
          showPanicMode={showPanicMode}
          panicMode={state.panicMode}
          onTogglePanicMode={onTogglePanicMode}
        />
      </div>
    );
  }

  return (
    <div className="panel-host">
      <SetupPanel
        ip={state.localIp}
        clients={state.clients}
        showPanicMode={showPanicMode}
        panicMode={state.panicMode}
        onTogglePanicMode={onTogglePanicMode}
      />
    </div>
  );
};

interface SetupProps {
  ip: string;
  clients: AppState["clients"];
  showPanicMode: boolean;
  panicMode: boolean;
  onTogglePanicMode: () => void;
}

const SetupPanel: React.FC<SetupProps> = ({
  ip,
  clients,
  showPanicMode,
  panicMode,
  onTogglePanicMode,
}) => {
  const [copied, setCopied] = useState(false);
  const fullAddr = `${ip}:25565`;

  const handleCopy = () => {
    if (!ip) return;
    navigator.clipboard.writeText(fullAddr).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  const anyOnline = clients.primary.online || clients.secondary.online;
  const subtitle = anyOnline
    ? "Ожидание второго клиента"
    : "Ожидание клиентов";

  return (
    <div className="panel">
      <div className="panel__header">
        <div className="panel__title">Запущено</div>
        <div className="panel__subtitle">{subtitle}</div>
      </div>

      <div className="field-row">
        <span className="field-row__label">Адрес</span>
        <span className="running-setup__addr">
          <span className="running-setup__addr-text">{fullAddr}</span>
          <button
            className={`icon-btn ${copied ? "is-success" : ""}`}
            onClick={handleCopy}
            title="Скопировать"
          >
            {copied ? <CheckIcon /> : <CopyIcon />}
          </button>
        </span>
      </div>

      <div className="field-row">
        <span className="field-row__label">Клиенты</span>
        <span className="client-list">
          <span className="client-list__item">
            <span
              className={`client-dot ${clients.primary.online ? "client-dot--online" : "client-dot--waiting"}`}
            />
            Основной
          </span>
          <span className="client-list__item">
            <span
              className={`client-dot ${clients.secondary.online ? "client-dot--online" : "client-dot--waiting"}`}
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

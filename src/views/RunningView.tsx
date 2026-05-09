import React, { useEffect, useState } from "react";
import { AppState } from "../types";
import { ActiveSession } from "../components/ActiveSession";
import { CheckIcon, CopyIcon } from "../components/Icons";

interface Props {
  state: AppState;
  onTogglePanicMode: () => void;
}

export const RunningView: React.FC<Props> = ({ state, onTogglePanicMode }) => {
  const anyOnline =
    state.clients.primary.online || state.clients.secondary.online;
  const bothOnline =
    state.clients.primary.online && state.clients.secondary.online;

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

  if (state.mode === "auto") {
    return (
      <div className="panel-host">
        <AutoSetupSteps
          clients={state.clients}
          panicMode={state.panicMode}
          onTogglePanicMode={onTogglePanicMode}
        />
      </div>
    );
  }

  return (
    <div className="panel-host">
      <ManualSetupPanel ip={state.localIp} clients={state.clients} />
    </div>
  );
};

interface ManualSetupProps {
  ip: string;
  clients: AppState["clients"];
}

const ManualSetupPanel: React.FC<ManualSetupProps> = ({ ip, clients }) => {
  const fullAddr = `${ip}:25565`;
  const anyOnline = clients.primary.online || clients.secondary.online;
  const subtitle = anyOnline ? "Ожидание второго клиента" : "Ожидание клиентов";

  return (
    <div className="panel">
      <div className="panel__header">
        <div className="panel__title">Запущено</div>
        <div className="panel__subtitle">{subtitle}</div>
      </div>

      <div className="field-row">
        <span className="field-row__label">Адрес</span>
        <CopyableAddr addr={fullAddr} disabled={!ip} fontSize={16} />
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
    </div>
  );
};

interface AutoSetupProps {
  clients: AppState["clients"];
  panicMode: boolean;
  onTogglePanicMode: () => void;
}

const AutoSetupSteps: React.FC<AutoSetupProps> = ({
  clients,
  panicMode,
  onTogglePanicMode,
}) => {
  const secondaryOnline = clients.secondary.online;
  const primaryOnline = clients.primary.online;

  return (
    <div className="panel">
      <div className="panel__header">
        <div className="panel__title">Запущено</div>
        <div className="panel__subtitle">Подключите оба клиента по шагам</div>
      </div>

      <SetupStep
        index={1}
        title="Дополнительный клиент"
        online={secondaryOnline}
      >
        <p className="setup-step__text">
          На втором устройстве (которое подключено к Wi-Fi-хотспоту этого ПК)
          зайдите на сервер как обычно (mc.funtime.su).
        </p>
      </SetupStep>

      <SetupStep index={2} title="Основной клиент" online={primaryOnline}>
        <p className="setup-step__text">
          В Minecraft на этом ПК подключитесь к адресу:
        </p>
        <CopyableAddr addr="127.0.0.1:25565" fontSize={15} />
      </SetupStep>

      <button
        className={`panic-mode ${panicMode ? "panic-mode--on" : "panic-mode--off"}`}
        onClick={onTogglePanicMode}
      >
        Нажми когда будешь на проверке
      </button>
    </div>
  );
};

interface SetupStepProps {
  index: number;
  title: string;
  online: boolean;
  children: React.ReactNode;
}

const SetupStep: React.FC<SetupStepProps> = ({
  index,
  title,
  online,
  children,
}) => (
  <div className={`setup-step ${online ? "setup-step--done" : ""}`}>
    <div className="setup-step__header">
      <span
        className={`client-dot ${online ? "client-dot--online" : "client-dot--waiting"}`}
      />
      <span className="setup-step__index">Шаг {index}</span>
      <span className="setup-step__title">{title}</span>
      <span className="setup-step__status">
        {online ? "подключён" : "ждём"}
      </span>
    </div>
    <div className="setup-step__body">{children}</div>
  </div>
);

interface CopyableAddrProps {
  addr: string;
  disabled?: boolean;
  fontSize?: number;
}

const CopyableAddr: React.FC<CopyableAddrProps> = ({
  addr,
  disabled,
  fontSize,
}) => {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    if (disabled) return;
    navigator.clipboard.writeText(addr).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <span className="running-setup__addr">
      <span
        className="running-setup__addr-text"
        style={fontSize ? { fontSize: `${fontSize}px` } : undefined}
      >
        {addr}
      </span>
      <button
        className={`icon-btn ${copied ? "is-success" : ""}`}
        onClick={handleCopy}
        disabled={disabled}
        title="Скопировать"
      >
        {copied ? <CheckIcon /> : <CopyIcon />}
      </button>
    </span>
  );
};

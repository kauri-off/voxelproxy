import React, { useState } from "react";
import { ClientStatus } from "../types";
import { CheckIcon, CopyIcon } from "./Icons";

interface Props {
  ip: string;
  clients: {
    primary: ClientStatus;
    secondary: ClientStatus;
  };
  nickName: string;
}

export const ConnectInstruction: React.FC<Props> = ({
  ip,
  clients,
  nickName,
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

  const allOnline = clients.primary.online && clients.secondary.online;
  const anyOnline = clients.primary.online || clients.secondary.online;

  return (
    <div className="connect-instruction">
      <div className="connect-instruction__label">
        {allOnline
          ? `Подключено: ${nickName}`
          : anyOnline
            ? "Ожидание 2го клиента"
            : "Подключите Minecraft клиенты к:"}
      </div>
      <div className="connect-instruction__addr-row">
        <div className="connect-instruction__addr">{fullAddr}</div>
        <button
          className={`connect-instruction__copy-btn ${copied ? "copied" : ""}`}
          onClick={handleCopy}
          title="Скопировать"
        >
          {copied ? <CheckIcon /> : <CopyIcon />}
        </button>
      </div>
    </div>
  );
};

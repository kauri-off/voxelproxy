import React, { useState } from "react";
import { Phase } from "../types";
import { CheckIcon, CopyIcon } from "./Icons";

interface Props {
  ip: string;
  phase: Phase;
}

export const ConnectInstruction: React.FC<Props> = ({ ip, phase }) => {
  const [copied, setCopied] = useState(false);
  const fullAddr = `${ip}:25565`;

  const handleCopy = () => {
    if (!ip) return;
    navigator.clipboard.writeText(fullAddr).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <div className="connect-instruction">
      <div className="connect-instruction__label">
        {phase === "active"
          ? "Подключено к:"
          : "Подключите Minecraft клиентов к:"}
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

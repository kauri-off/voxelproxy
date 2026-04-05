import React from "react";
import { AppState } from "../types";
import { ClientCard } from "../components/ClientCard";
import { ConnectInstruction } from "../components/ConnectInstruction";

interface Props {
  state: AppState;
}

export const RunningView: React.FC<Props> = ({ state }) => {
  return (
    <div className="running-view__body">
      <ConnectInstruction ip={state.localIp} phase={state.phase} />

      <div className="client-cards">
        <ClientCard type="primary" online={state.clients.primary.online} />
        <ClientCard type="secondary" online={state.clients.secondary.online} />
      </div>
    </div>
  );
};

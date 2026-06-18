import React from "react";
import { AppState } from "../types";
import { commands } from "../bindings";
import { MailIcon } from "./Icons";

interface Props {
  state: AppState;
  onStop: () => void;
  onContact: () => void;
}

export const TitleBar: React.FC<Props> = ({ state, onStop, onContact }) => (
  <header className="titlebar">
    <button
      className="titlebar__logo"
      onClick={() =>
        commands.openUrl("https://github.com/kauri-off/voxelproxy")
      }
    >
      VoxelProxy
    </button>
    <span className="titlebar__version">
      {state.version ? `v${state.version}` : ""}
    </span>
    <span className="titlebar__spacer" />
    <button
      className="titlebar__contact"
      onClick={onContact}
      title="Сообщение разработчику"
      aria-label="Сообщение разработчику"
    >
      <MailIcon />
    </button>
    {state.phase !== "idle" && (
      <button className="titlebar__stop" onClick={onStop}>
        ■ Остановить
      </button>
    )}
  </header>
);

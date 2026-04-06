import React from "react";
import { AppState } from "../types";
import { openUrl } from "../tauri";

interface Props {
  state: AppState;
  onStop: () => void;
}

export const TitleBar: React.FC<Props> = ({ state, onStop }) => (
  <header className="titlebar">
    <button
      className="titlebar__logo"
      onClick={() => openUrl("https://github.com/kauri-off/voxelproxy")}
    >
      VoxelProxy
    </button>
    <span className="titlebar__version">
      {state.version ? `v${state.version}` : ""}
    </span>
    <span className="titlebar__spacer" />
    {state.phase !== "idle" && (
      <button className="titlebar__stop" onClick={onStop}>
        ■ Остановить
      </button>
    )}
  </header>
);

import ReactDOM from "react-dom/client";
import { App } from "./App";
import "./styles/index.css";

// Disable the default WebView2 context menu (Back / Refresh / Save as / Print / ...).
// Keep it for editable fields so right-click copy/paste still works there.
document.addEventListener("contextmenu", (e) => {
  const target = e.target as HTMLElement | null;
  const isEditable =
    target?.isContentEditable ||
    target?.closest("input, textarea, [contenteditable='true']") != null;
  if (!isEditable) {
    e.preventDefault();
  }
});

ReactDOM.createRoot(document.getElementById("root")!).render(<App />);

import './styles/index.css';

import { AppStore } from './state.js';
import { TitleBar } from './components/TitleBar.js';
import { LogPanel } from './components/LogPanel.js';
import { IdleView } from './views/IdleView.js';
import { RunningView } from './views/RunningView.js';
import {
  getVersion,
  getLocalIpAddr,
  checkUpdates,
  onProxyLog,
  onSessionStarted,
  onSessionEnded,
  onClientStatus,
} from './tauri.js';

async function main(): Promise<void> {
  const store = new AppStore();

  // ── Components ──────────────────────────────────────────────────────────────
  const titleBar = new TitleBar();
  const logPanel = new LogPanel();
  const idleView = new IdleView(store, logPanel);
  const runningView = new RunningView();

  // ── Build DOM ───────────────────────────────────────────────────────────────
  const app = document.createElement('div');
  app.className = 'app';

  const viewContainer = document.createElement('div');
  viewContainer.className = 'view-container';
  viewContainer.append(idleView.render(), runningView.render());

  app.append(titleBar.render(), viewContainer, logPanel.render());
  document.body.appendChild(app);

  // Initial view
  idleView.show();

  // ── Subscribe to store changes ───────────────────────────────────────────────
  store.subscribe(state => {
    titleBar.update(state);

    if (state.phase === 'idle') {
      runningView.hide();
      idleView.show();
    } else {
      idleView.hide();
      runningView.show(state);
    }
  });

  // ── Tauri events ─────────────────────────────────────────────────────────────
  await onProxyLog(entry => {
    logPanel.addEntry(entry);
  });

  await onSessionStarted(() => {
    store.setState({
      phase: 'waiting',
      clients: { primary: { online: false }, secondary: { online: false } },
    });
  });

  await onSessionEnded(() => {
    store.setState({ phase: 'idle' });
  });

  await onClientStatus(({ which, online }) => {
    const prev = store.getState();
    const clients = {
      ...prev.clients,
      [which]: { online },
    };
    const anyOnline = clients.primary.online || clients.secondary.online;
    const newPhase = anyOnline && prev.phase !== 'idle' ? 'active' : prev.phase;
    store.setState({ clients, phase: newPhase });
  });

  // ── Initial data fetch ───────────────────────────────────────────────────────
  try {
    const version = await getVersion();
    store.setState({ version });
  } catch (_) {}

  try {
    const ip = await getLocalIpAddr();
    store.setState({ localIp: ip });
  } catch (_) {}

  try {
    const updateInfo = await checkUpdates();
    if (updateInfo) store.setState({ updateInfo });
  } catch (_) {}
}

main();

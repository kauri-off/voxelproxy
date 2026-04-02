import type { AppMode } from '../state.js';
import { AppStore } from '../state.js';
import { startManualSession, startAutoSession } from '../tauri.js';
import type { LogPanel } from '../components/LogPanel.js';

export class IdleView {
  private el: HTMLElement;
  private serverAddrInput: HTMLInputElement;
  private serverSection: HTMLElement;
  private autoSection: HTMLElement;
  private windivertCheck: HTMLInputElement;
  private portRangeRow: HTMLElement;
  private portMinInput: HTMLInputElement;
  private portMaxInput: HTMLInputElement;
  private startBtn: HTMLButtonElement;
  private tabManual: HTMLButtonElement;
  private tabAuto: HTMLButtonElement;

  constructor(
    private store: AppStore,
    private logPanel: LogPanel,
  ) {
    this.el = document.createElement('div');
    this.el.className = 'view';

    const body = document.createElement('div');
    body.className = 'idle-view__body';

    const card = document.createElement('div');
    card.className = 'config-card';

    // Mode tabs
    const tabsSection = document.createElement('div');
    tabsSection.className = 'config-card__section';

    const tabsLabel = document.createElement('div');
    tabsLabel.className = 'section-label';
    tabsLabel.textContent = 'Режим';

    const tabs = document.createElement('div');
    tabs.className = 'mode-tabs';

    this.tabManual = document.createElement('button');
    this.tabManual.className = 'tab active';
    this.tabManual.textContent = 'Ручной';
    this.tabManual.addEventListener('click', () => this.setMode('manual'));

    this.tabAuto = document.createElement('button');
    this.tabAuto.className = 'tab';
    this.tabAuto.textContent = 'Авто';
    this.tabAuto.addEventListener('click', () => this.setMode('auto'));

    tabs.append(this.tabManual, this.tabAuto);
    tabsSection.append(tabsLabel, tabs);

    // Server address (manual)
    this.serverSection = document.createElement('div');
    this.serverSection.className = 'config-card__section';

    const serverLabel = document.createElement('div');
    serverLabel.className = 'section-label';
    serverLabel.textContent = 'Адрес сервера';

    this.serverAddrInput = document.createElement('input');
    this.serverAddrInput.type = 'text';
    this.serverAddrInput.className = 'text-input';
    this.serverAddrInput.placeholder = 'mc.example.com';
    this.serverAddrInput.spellcheck = false;
    this.serverAddrInput.addEventListener('input', () => {
      this.store.setState({ manualServerAddr: this.serverAddrInput.value });
    });
    this.serverAddrInput.addEventListener('keydown', e => {
      if (e.key === 'Enter') this.handleStart();
    });

    this.serverSection.append(serverLabel, this.serverAddrInput);

    // Auto options
    this.autoSection = document.createElement('div');
    this.autoSection.className = 'config-card__section';
    this.autoSection.hidden = true;

    const winLabel = document.createElement('label');
    winLabel.className = 'checkbox-label';

    this.windivertCheck = document.createElement('input');
    this.windivertCheck.type = 'checkbox';
    this.windivertCheck.checked = true;
    this.windivertCheck.addEventListener('change', () => {
      this.store.setState({ autoUseWindivert: this.windivertCheck.checked });
      this.togglePortRange();
    });

    const winText = document.createElement('span');
    winText.textContent = 'WinDivert (перехват хотспота)';
    winLabel.append(this.windivertCheck, winText);

    this.portRangeRow = document.createElement('div');
    this.portRangeRow.className = 'port-range-row';

    const hint = document.createElement('span');
    hint.className = 'hint';
    hint.textContent = 'Порты:';

    this.portMinInput = document.createElement('input');
    this.portMinInput.type = 'number';
    this.portMinInput.className = 'port-input';
    this.portMinInput.value = '25560';
    this.portMinInput.min = '1';
    this.portMinInput.max = '65534';
    this.portMinInput.addEventListener('input', () => {
      this.store.setState({ autoPortMin: parseInt(this.portMinInput.value, 10) });
    });

    const dash = document.createElement('span');
    dash.className = 'hint';
    dash.textContent = '–';

    this.portMaxInput = document.createElement('input');
    this.portMaxInput.type = 'number';
    this.portMaxInput.className = 'port-input';
    this.portMaxInput.value = '25570';
    this.portMaxInput.min = '2';
    this.portMaxInput.max = '65535';
    this.portMaxInput.addEventListener('input', () => {
      this.store.setState({ autoPortMax: parseInt(this.portMaxInput.value, 10) });
    });

    this.portRangeRow.append(hint, this.portMinInput, dash, this.portMaxInput);
    this.autoSection.append(winLabel, this.portRangeRow);

    // Start button
    this.startBtn = document.createElement('button');
    this.startBtn.className = 'btn-primary';
    this.startBtn.textContent = '▶  Запустить сессию';
    this.startBtn.addEventListener('click', () => this.handleStart());

    card.append(tabsSection, this.serverSection, this.autoSection, this.startBtn);
    body.appendChild(card);
    this.el.append(body);
  }

  render(): HTMLElement {
    return this.el;
  }

  show(): void {
    this.el.classList.add('is-visible');
    // Restore state from store
    const s = this.store.getState();
    this.serverAddrInput.value = s.manualServerAddr;
    this.windivertCheck.checked = s.autoUseWindivert;
    this.portMinInput.value = String(s.autoPortMin);
    this.portMaxInput.value = String(s.autoPortMax);
    this.applyMode(s.mode);
    this.togglePortRange();
    this.setControlsDisabled(false);
  }

  hide(): void {
    this.el.classList.remove('is-visible');
  }

  private setMode(mode: AppMode): void {
    this.store.setState({ mode });
    this.applyMode(mode);
  }

  private applyMode(mode: AppMode): void {
    this.tabManual.classList.toggle('active', mode === 'manual');
    this.tabAuto.classList.toggle('active', mode === 'auto');
    this.serverSection.hidden = mode !== 'manual';
    this.autoSection.hidden = mode !== 'auto';
  }

  private togglePortRange(): void {
    this.portRangeRow.style.display = this.windivertCheck.checked ? 'flex' : 'none';
  }

  private setControlsDisabled(disabled: boolean): void {
    this.tabManual.disabled = disabled;
    this.tabAuto.disabled = disabled;
    this.serverAddrInput.disabled = disabled;
    this.windivertCheck.disabled = disabled;
    this.portMinInput.disabled = disabled;
    this.portMaxInput.disabled = disabled;
    this.startBtn.disabled = disabled;
  }

  private async handleStart(): Promise<void> {
    const state = this.store.getState();

    if (state.mode === 'manual') {
      const addr = this.serverAddrInput.value.trim();
      if (!addr) {
        this.logPanel.addEntry({ level: 'error', message: 'Введите адрес сервера' });
        return;
      }
      this.store.setState({ manualServerAddr: addr });
      this.setControlsDisabled(true);
      try {
        await startManualSession(addr);
        this.store.setState({ phase: 'waiting', clients: { primary: { online: false }, secondary: { online: false } } });
      } catch (err) {
        this.logPanel.addEntry({ level: 'error', message: String(err) });
        this.setControlsDisabled(false);
      }
    } else {
      const useWindivert = this.windivertCheck.checked;
      let portMin = state.autoPortMin;
      let portMax = state.autoPortMax;

      if (useWindivert) {
        portMin = parseInt(this.portMinInput.value, 10);
        portMax = parseInt(this.portMaxInput.value, 10);
        if (isNaN(portMin) || isNaN(portMax) || portMin < 1 || portMax > 65535 || portMin >= portMax) {
          this.logPanel.addEntry({ level: 'error', message: 'Неверный диапазон портов (min < max, 1–65535)' });
          return;
        }
      }

      this.setControlsDisabled(true);
      try {
        await startAutoSession(useWindivert, portMin, portMax);
        this.store.setState({ phase: 'waiting', clients: { primary: { online: false }, secondary: { online: false } } });
      } catch (err) {
        this.logPanel.addEntry({ level: 'error', message: String(err) });
        this.setControlsDisabled(false);
      }
    }
  }
}

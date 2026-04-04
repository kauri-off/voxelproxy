import type { AppState } from '../state.js';
import { stopSession, openUrl } from '../tauri.js';

export class TitleBar {
  private el: HTMLElement;
  private versionEl: HTMLElement;
  private updateEl: HTMLAnchorElement;
  private stopBtn: HTMLButtonElement;

  constructor() {
    this.el = document.createElement('header');
    this.el.className = 'titlebar';

    const logo = document.createElement('a');
    logo.className = 'titlebar__logo';
    logo.textContent = 'VoxelProxy';
    logo.href = '#';
    logo.addEventListener('click', e => {
      e.preventDefault();
      openUrl('https://github.com/kauri-off/voxelproxy').catch(() => {});
    });

    this.versionEl = document.createElement('span');
    this.versionEl.className = 'titlebar__version';

    const spacer = document.createElement('span');
    spacer.className = 'titlebar__spacer';

    this.stopBtn = document.createElement('button');
    this.stopBtn.className = 'titlebar__stop';
    this.stopBtn.textContent = '■ Остановить';
    this.stopBtn.hidden = true;
    this.stopBtn.addEventListener('click', () => {
      stopSession().catch(() => {});
    });

    this.updateEl = document.createElement('a');
    this.updateEl.className = 'titlebar__update';
    this.updateEl.href = '#';
    this.updateEl.hidden = true;
    this.updateEl.addEventListener('click', e => {
      e.preventDefault();
      const state = this.updateEl.dataset['href'];
      if (state) openUrl(state).catch(() => {});
    });

    this.el.append(logo, this.versionEl, spacer, this.stopBtn, this.updateEl);
  }

  render(): HTMLElement {
    return this.el;
  }

  update(state: AppState): void {
    this.versionEl.textContent = state.version ? `v${state.version}` : '';
    this.stopBtn.hidden = state.phase === 'idle';

    if (state.updateInfo) {
      this.updateEl.textContent = `Доступно обновление ${state.updateInfo.tag}`;
      this.updateEl.dataset['href'] = state.updateInfo.link;
      this.updateEl.hidden = false;
    } else {
      this.updateEl.hidden = true;
    }
  }
}

import type { SessionPhase } from '../state.js';

export class ConnectInstruction {
  private el: HTMLElement;
  private labelEl: HTMLElement;
  private addrRow: HTMLElement;
  private addrEl: HTMLElement;
  private copyBtn: HTMLButtonElement;
  private ip = '';

  constructor() {
    this.el = document.createElement('div');
    this.el.className = 'connect-instruction';

    this.labelEl = document.createElement('div');
    this.labelEl.className = 'connect-instruction__label';
    this.labelEl.textContent = 'Подключите Minecraft клиентов к:';

    this.addrRow = document.createElement('div');
    this.addrRow.className = 'connect-instruction__addr-row';

    this.addrEl = document.createElement('div');
    this.addrEl.className = 'connect-instruction__addr';

    this.copyBtn = document.createElement('button');
    this.copyBtn.className = 'connect-instruction__copy-btn';
    this.copyBtn.title = 'Скопировать';
    this.copyBtn.innerHTML =
      '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">' +
      '<rect x="9" y="9" width="13" height="13" rx="2"/>' +
      '<path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>' +
      '</svg>';
    this.copyBtn.addEventListener('click', () => this.handleCopy());

    this.addrRow.append(this.addrEl, this.copyBtn);
    this.el.append(this.labelEl, this.addrRow);
  }

  render(): HTMLElement {
    return this.el;
  }

  update(ip: string, phase: SessionPhase): void {
    this.ip = ip;
    this.addrEl.textContent = `${ip}:25565`;
    this.labelEl.textContent =
      phase === 'active'
        ? 'Подключено к:'
        : 'Подключите Minecraft клиентов к:';
  }

  private handleCopy(): void {
    if (!this.ip) return;
    navigator.clipboard.writeText(`${this.ip}:25565`).then(() => {
      const prev = this.copyBtn.innerHTML;
      this.copyBtn.innerHTML =
        '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">' +
        '<polyline points="20 6 9 17 4 12"/>' +
        '</svg>';
      this.copyBtn.classList.add('copied');
      setTimeout(() => {
        this.copyBtn.innerHTML = prev;
        this.copyBtn.classList.remove('copied');
      }, 1500);
    });
  }
}

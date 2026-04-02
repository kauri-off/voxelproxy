export type ClientWhich = 'primary' | 'secondary';

const LABELS: Record<ClientWhich, string> = {
  primary: 'Основной',
  secondary: 'Дополнительный',
};

export class ClientCard {
  private el: HTMLElement;
  private indicator: HTMLElement;
  private statusEl: HTMLElement;
  private online = false;

  constructor(which: ClientWhich) {
    this.el = document.createElement('div');
    this.el.className = 'client-card client-card--waiting';

    this.indicator = document.createElement('div');
    this.indicator.className = 'client-card__indicator';

    const name = document.createElement('div');
    name.className = 'client-card__name';
    name.textContent = LABELS[which];

    this.statusEl = document.createElement('div');
    this.statusEl.className = 'client-card__status';
    this.statusEl.textContent = 'Ожидание…';

    this.el.append(this.indicator, name, this.statusEl);
  }

  render(): HTMLElement {
    return this.el;
  }

  update(online: boolean): void {
    if (this.online === online) return;
    this.online = online;

    this.el.classList.toggle('client-card--waiting', !online);
    this.el.classList.toggle('client-card--online', online);
    this.statusEl.textContent = online ? 'Подключён' : 'Ожидание…';
  }

  reset(): void {
    this.update(false);
  }
}

import type { AppState } from '../state.js';
import { ClientCard } from '../components/ClientCard.js';
import { ConnectInstruction } from '../components/ConnectInstruction.js';

export class RunningView {
  private el: HTMLElement;
  private instruction: ConnectInstruction;
  private primaryCard: ClientCard;
  private secondaryCard: ClientCard;

  constructor() {
    this.el = document.createElement('div');
    this.el.className = 'view';

    const body = document.createElement('div');
    body.className = 'running-view__body';

    this.instruction = new ConnectInstruction();

    const cardsRow = document.createElement('div');
    cardsRow.className = 'client-cards';

    this.primaryCard = new ClientCard('primary');
    this.secondaryCard = new ClientCard('secondary');

    cardsRow.append(this.primaryCard.render(), this.secondaryCard.render());
    body.append(this.instruction.render(), cardsRow);
    this.el.appendChild(body);
  }

  render(): HTMLElement {
    return this.el;
  }

  show(state: AppState): void {
    this.update(state);
    this.el.classList.add('is-visible');
  }

  hide(): void {
    this.el.classList.remove('is-visible');
    this.primaryCard.reset();
    this.secondaryCard.reset();
  }

  update(state: AppState): void {
    this.instruction.update(state.localIp, state.phase);
    this.primaryCard.update(state.clients.primary.online);
    this.secondaryCard.update(state.clients.secondary.online);
  }
}

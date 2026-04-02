import type { LogEntry } from '../tauri.js';

const PREFIXES: Record<string, string> = {
  success: '[+]',
  info:    '[~]',
  warn:    '[!]',
  error:   '[!]',
};

export class LogPanel {
  private el: HTMLElement;
  private bodyEl: HTMLElement;
  private countEl: HTMLElement;
  private expanded = false;
  private entryCount = 0;

  constructor() {
    this.el = document.createElement('div');
    this.el.className = 'log-drawer';

    const header = document.createElement('div');
    header.className = 'log-drawer__header';
    header.addEventListener('click', e => {
      // Don't toggle when clicking the clear button
      if ((e.target as HTMLElement).closest('.log-drawer__clear')) return;
      this.setExpanded(!this.expanded);
    });

    const chevron = document.createElement('span');
    chevron.className = 'log-drawer__chevron';
    chevron.textContent = '▲';

    const label = document.createElement('span');
    label.className = 'log-drawer__label';
    label.textContent = 'Лог';

    this.countEl = document.createElement('span');
    this.countEl.className = 'log-drawer__count';

    const spacer = document.createElement('span');
    spacer.className = 'log-drawer__spacer';

    const clearBtn = document.createElement('button');
    clearBtn.className = 'log-drawer__clear';
    clearBtn.textContent = 'Очистить';
    clearBtn.addEventListener('click', () => this.clear());

    header.append(chevron, label, this.countEl, spacer, clearBtn);

    this.bodyEl = document.createElement('div');
    this.bodyEl.className = 'log-drawer__body';

    this.el.append(header, this.bodyEl);
  }

  render(): HTMLElement {
    return this.el;
  }

  addEntry(entry: LogEntry): void {
    const prefix = PREFIXES[entry.level] ?? '[?]';
    const div = document.createElement('div');
    div.className = `log-entry ${entry.level}`;
    div.textContent = `${prefix} ${entry.message}`;
    this.bodyEl.appendChild(div);
    this.bodyEl.scrollTop = this.bodyEl.scrollHeight;

    this.entryCount++;
    this.countEl.textContent = `(${this.entryCount})`;
  }

  clear(): void {
    this.bodyEl.innerHTML = '';
    this.entryCount = 0;
    this.countEl.textContent = '';
  }

  setExpanded(expanded: boolean): void {
    this.expanded = expanded;
    this.el.classList.toggle('is-expanded', expanded);
    if (expanded) {
      this.bodyEl.scrollTop = this.bodyEl.scrollHeight;
    }
  }
}

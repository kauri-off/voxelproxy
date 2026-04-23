import { useEffect } from "react";
import { ChangelogEntry } from "../bindings";

interface Props {
  entries: ChangelogEntry[];
  onDismiss: () => void;
}

export const ChangelogModal: React.FC<Props> = ({ entries, onDismiss }) => {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onDismiss();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onDismiss]);

  if (entries.length === 0) return null;

  const newest = entries[0].version;
  const oldest = entries[entries.length - 1].version;
  const title =
    entries.length > 1 ? (
      <>
        Обновление <span className="from">{oldest}</span>
        <span className="arrow">→</span>
        <span className="to">{newest}</span>
      </>
    ) : (
      <>
        Что нового в <span className="to">{newest}</span>
      </>
    );

  return (
    <div
      className="changelog-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) onDismiss();
      }}
    >
      <div className="changelog-card" role="dialog" aria-modal="true">
        <div className="changelog-card__header">
          <span className="changelog-card__title">{title}</span>
          <button
            className="changelog-card__close"
            onClick={onDismiss}
            aria-label="Закрыть"
          >
            ✕
          </button>
        </div>

        <div className="changelog-card__body">
          {entries.map((entry) => (
            <section className="changelog-section" key={entry.version}>
              <div className="changelog-section__version">v{entry.version}</div>
              <div
                className="changelog-body"
                dangerouslySetInnerHTML={{ __html: entry.html }}
              />
            </section>
          ))}
        </div>

        <div className="changelog-card__footer">
          <button className="changelog-card__ok" onClick={onDismiss}>
            Понятно
          </button>
        </div>
      </div>
    </div>
  );
};

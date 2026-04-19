import React, { useState, useEffect, useRef } from "react";
import { ProxyLogEvent } from "../bindings";

export const LogPanel: React.FC<{
  logs: ProxyLogEvent[];
  onClear: () => void;
}> = ({ logs, onClear }) => {
  const [isExpanded, setIsExpanded] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const prevErrorCount = useRef(0);

  useEffect(() => {
    const currentErrorCount = logs.filter(
      (log) => log.level === "Error",
    ).length;
    if (currentErrorCount > prevErrorCount.current) {
      setIsExpanded(true);
    }
    prevErrorCount.current = currentErrorCount;
  }, [logs]);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [logs, isExpanded]);

  return (
    <div className={`log-drawer ${isExpanded ? "is-expanded" : ""}`}>
      <div
        className="log-drawer__header"
        onClick={() => setIsExpanded(!isExpanded)}
        role="button"
        tabIndex={0}
        onKeyDown={(e) =>
          (e.key === "Enter" || e.key === " ") && setIsExpanded(!isExpanded)
        }
      >
        <span className="log-drawer__label">Лог</span>
        <span className="log-drawer__count">({logs.length})</span>
        <div className="log-drawer__spacer" />
        <button
          className="log-drawer__clear"
          onClick={(e) => {
            e.stopPropagation();
            onClear();
          }}
        >
          Очистить
        </button>
        <span className="log-drawer__chevron">▼</span>
      </div>
      <div className="log-drawer__body" ref={scrollRef}>
        {logs.map((log, i) => (
          <div key={i} className={`log-entry ${log.level}`}>
            {log.message}
          </div>
        ))}
      </div>
    </div>
  );
};

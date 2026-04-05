import React from "react";

interface Props {
  type: "primary" | "secondary";
  online: boolean;
}

export const ClientCard: React.FC<Props> = ({ type, online }) => {
  const statusClass = online ? "client-card--online" : "client-card--waiting";

  return (
    <div className={`client-card ${statusClass}`}>
      <div className="client-card__indicator" />
      <div className="client-card__name">
        {type === "primary" ? "Основное устройство" : "Второе устройство"}
      </div>
      <div className="client-card__status">
        {online ? "Подключено" : "Ожидание..."}
      </div>
    </div>
  );
};

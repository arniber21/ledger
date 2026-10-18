import type { ReactNode } from "react";
import { useShell } from "./context";

export function Header({
  icon,
  title,
  meta,
  actions,
}: {
  icon: ReactNode;
  title: string;
  meta?: string;
  actions?: ReactNode;
}) {
  const { metrics } = useShell();
  const live = metrics.connection === "live";

  return (
    <header className="header">
      <div className="crumb">
        <span className="crumb-icon">{icon}</span>
        <h1>{title}</h1>
        {meta ? <span className="crumb-meta">{meta}</span> : null}
      </div>
      <div className="header-actions">
        {actions}
        <span className="company">Hearthloaf Bakeries</span>
        <span className={live ? "presence is-live" : "presence"} data-testid="connection">
          <span className="presence-dot" />
          {live ? "Live" : "Offline"}
        </span>
      </div>
    </header>
  );
}

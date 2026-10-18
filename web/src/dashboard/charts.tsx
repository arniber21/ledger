import type { AttentionPoint, MonitorPoint, RatePoint } from "../stream/types";

const WIDTH = 520;
const HEIGHT = 132;

function formatTick(value: number): string {
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 }).format(value);
}

function polyline(values: number[], max: number): string {
  if (values.length === 0) return "";
  return values
    .map((value, index) => {
      const x = values.length === 1 ? WIDTH / 2 : (index / (values.length - 1)) * (WIDTH - 72);
      const y = HEIGHT - 16 - (value / max) * (HEIGHT - 28);
      return `${x},${y}`;
    })
    .join(" ");
}

function lastY(value: number, max: number): number {
  return HEIGHT - 16 - (value / max) * (HEIGHT - 28);
}

export function EventsChart({ points }: { points: RatePoint[] }) {
  const agent = points.map((point) => point.agentLog);
  const exchange = points.map((point) => point.exchange);
  const solana = points.map((point) => point.solana);
  const max = Math.max(1, ...agent, ...exchange, ...solana);
  const last = points.at(-1);

  return (
    <section className="chart" data-testid="chart-events">
      <div className="chart-head">
        <div className="chart-copy">
          <h2>Events per second</h2>
          <p>Last 10 minutes, by source</p>
        </div>
      </div>
      {points.length === 0 || max === 1 && [...agent, ...exchange, ...solana].every((value) => value === 0) ? (
        <ChartEmpty message={points.length === 0 ? undefined : "No events in this window"} />
      ) : (
        <div className="plot">
          <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} role="img" aria-label="Events per second by source">
            <Axis />
            <polyline points={polyline(agent, max)} className="line line-accent" />
            <polyline points={polyline(exchange, max)} className="line line-ink" />
            <polyline points={polyline(solana, max)} className="line line-grey" />
          </svg>
          {last ? (
            <>
              <span className="end-label end-accent" style={{ top: lastY(last.agentLog, max) }}>
                Agent log {formatTick(last.agentLog)}
              </span>
              <span className="end-label end-ink" style={{ top: lastY(last.exchange, max) }}>
                Exchange {formatTick(last.exchange)}
              </span>
              <span className="end-label end-grey" style={{ top: lastY(last.solana, max) }}>
                Solana {formatTick(last.solana)}
              </span>
            </>
          ) : null}
          <AxisLabels />
        </div>
      )}
    </section>
  );
}

export function AttentionChart({ points }: { points: AttentionPoint[] }) {
  const max = Math.max(1, ...points.map((point) => point.violate + point.inconclusive));

  return (
    <section className="chart" data-testid="chart-attention">
      <div className="chart-head">
        <div>
          <h2>Needs attention per minute</h2>
        </div>
        <div className="legend">
          <span>
            <i className="swatch swatch-violations" /> Violate
          </span>
          <span>
            <i className="swatch swatch-unknowns" /> Inconclusive
          </span>
        </div>
      </div>
      {points.length === 0 || points.every((point) => point.violate + point.inconclusive === 0) ? (
        <ChartEmpty message={points.length === 0 ? undefined : "No verdicts in this window"} />
      ) : (
        <div className="plot plot-bars">
          <div className="bars" aria-label="Violate and inconclusive counts">
            {points.map((point) => (
              <div className="bar" key={point.t}>
                <span
                  className="bar-inconclusive"
                  style={{ height: `${(point.inconclusive / max) * 100}%` }}
                />
                <span className="bar-violate" style={{ height: `${(point.violate / max) * 100}%` }} />
              </div>
            ))}
          </div>
          <AxisLabels />
        </div>
      )}
    </section>
  );
}

export function MonitorsChart({ points }: { points: MonitorPoint[] }) {
  const withGc = points.map((point) => point.withGc);
  const without = points.map((point) => point.withoutGc);
  const max = Math.max(1, ...withGc, ...without);
  const last = points.at(-1);

  return (
    <section className="chart" data-testid="chart-monitors">
      <div className="chart-head">
        <div className="chart-copy">
          <h2>Live monitors</h2>
          <p>With and without GC</p>
        </div>
      </div>
      {points.length === 0 ? (
        <ChartEmpty />
      ) : (
        <div className="plot">
          <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} role="img" aria-label="Live monitors with and without GC">
            <Axis />
            <polyline points={polyline(without, max)} className="line line-grey line-dashed" />
            <polyline points={polyline(withGc, max)} className="line line-accent" />
          </svg>
          {last ? (
            <>
              {last.withoutGc !== last.withGc ? (
                <span className="end-label end-grey" style={{ top: lastY(last.withoutGc, max) }}>
                  Without GC {formatTick(last.withoutGc)}
                </span>
              ) : null}
              <span className="end-label end-accent" style={{ top: lastY(last.withGc, max) }}>
                With GC {formatTick(last.withGc)}
              </span>
            </>
          ) : null}
          <AxisLabels />
        </div>
      )}
    </section>
  );
}

function ChartEmpty({ message = "No samples yet" }: { message?: string }) {
  return (
    <div className="plot plot-empty">
      <p>{message}</p>
      <AxisLabels />
    </div>
  );
}

function Axis() {
  return <line x1="0" y1={HEIGHT - 16} x2={WIDTH - 78} y2={HEIGHT - 16} className="axis" />;
}

function AxisLabels() {
  return (
    <div className="axis-labels">
      <span>10 min ago</span>
      <span>Now</span>
    </div>
  );
}

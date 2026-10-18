import { tableFeatures, useTable, type ColumnDef } from "@tanstack/react-table";
import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  bandLabel,
  countLabel,
  groupCalibration,
  parseCalibration,
  reviewedLabel,
  SMALL_SAMPLE,
  type CalibrationRow,
  type CalibrationTable,
} from "./calibration";

const features = tableFeatures({});
type Features = typeof features;

const columns: ColumnDef<Features, CalibrationRow>[] = [
  { accessorKey: "band", header: "Score band" },
  { accessorKey: "confirmed", header: "Confirmed" },
  { accessorKey: "dismissed", header: "Dismissed" },
  { accessorKey: "reviewed", header: "Reviewed" },
];

async function fetchCalibration(): Promise<CalibrationTable> {
  const response = await fetch("/api/calibration");
  if (!response.ok) throw new Error("Calibration is unavailable.");
  const table = parseCalibration(await response.json());
  if (!table) throw new Error("Calibration is unavailable.");
  return table;
}

export function Calibration() {
  const query = useQuery({
    queryKey: ["calibration"],
    queryFn: fetchCalibration,
    retry: false,
    staleTime: 5_000,
  });

  return (
    <section className="calibration" aria-labelledby="calibration-title" data-testid="calibration">
      <div className="chart-copy">
        <h2 id="calibration-title">Calibration</h2>
        <p>
          Counts of inbox reviews by Jev score band. Confirmed means marked reviewed. A violate is
          needs review. Unknown is inconclusive, not a pass. The {query.data?.thresholdLabel ?? "0.80"}{" "}
          threshold stays fixed. Fewer than {SMALL_SAMPLE} reviews is a small sample.
        </p>
      </div>
      <CalibrationBody query={query} />
    </section>
  );
}

function CalibrationBody({
  query,
}: {
  query: { isPending: boolean; isError: boolean; data: CalibrationTable | undefined };
}) {
  if (query.isPending) return <p className="column-empty">Loading calibration.</p>;
  if (query.isError || !query.data) {
    return (
      <p className="column-empty" data-testid="calibration-unavailable">
        Calibration is unavailable.
      </p>
    );
  }
  if (groupCalibration(query.data.rows).length === 0) {
    return (
      <p className="column-empty" data-testid="calibration-empty">
        No reviews yet.
      </p>
    );
  }
  return <CalibrationGrid rows={query.data.rows} />;
}

function CalibrationGrid({ rows }: { rows: CalibrationRow[] }) {
  const groups = useMemo(() => groupCalibration(rows), [rows]);
  const visible = useMemo(() => groups.flatMap((group) => group.rows), [groups]);
  const table = useTable({
    features,
    data: visible,
    columns,
    getRowId: (row) => `${row.question}|${row.band}`,
  });
  const byId = new Map(table.getRowModel().rows.map((row) => [row.id, row]));

  return (
    <table className="spec-table calibration-table" data-testid="calibration-table">
      <thead>
        {table.getHeaderGroups().map((group) => (
          <tr key={group.id}>
            {group.headers.map((header) => (
              <th key={header.id} className={numeric(header.id) ? "num" : undefined}>
                <table.FlexRender header={header} />
              </th>
            ))}
          </tr>
        ))}
      </thead>
      {groups.map((group) => (
        <tbody key={group.question} data-testid="calibration-group">
          <tr className="calibration-group">
            <th scope="rowgroup" colSpan={4} title={group.question}>
              {group.label}
              <span>
                {countLabel(group.reviewed)} reviewed{group.reviewed < SMALL_SAMPLE ? " · small sample" : ""}
              </span>
            </th>
          </tr>
          {group.rows.map((item) => {
            const row = byId.get(`${item.question}|${item.band}`);
            if (!row) return null;
            return (
              <tr key={row.id} data-testid="calibration-row">
                <td>{bandLabel(row.original.band)}</td>
                <td className="num">{countLabel(row.original.confirmed)}</td>
                <td className="num">{countLabel(row.original.dismissed)}</td>
                <td className="num">{reviewedLabel(row.original.reviewed)}</td>
              </tr>
            );
          })}
        </tbody>
      ))}
    </table>
  );
}

function numeric(id: string): boolean {
  return id === "confirmed" || id === "dismissed" || id === "reviewed";
}

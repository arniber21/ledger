import { tableFeatures, useTable, type ColumnDef } from "@tanstack/react-table";
import { useQuery } from "@tanstack/react-query";
import {
  bandLabel,
  countLabel,
  parseCalibration,
  reviewedLabel,
  SMALL_SAMPLE,
  type CalibrationRow,
  type CalibrationTable,
} from "./calibration";

const features = tableFeatures({});
type Features = typeof features;

const columns: ColumnDef<Features, CalibrationRow>[] = [
  { accessorKey: "question", header: "Question" },
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
  if (query.data.rows.length === 0) {
    return (
      <p className="column-empty" data-testid="calibration-empty">
        No reviews yet.
      </p>
    );
  }
  return <CalibrationGrid rows={query.data.rows} />;
}

function CalibrationGrid({ rows }: { rows: CalibrationRow[] }) {
  const table = useTable({
    features,
    data: rows,
    columns,
    getRowId: (row) => `${row.question}|${row.band}`,
  });

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
      <tbody>
        {table.getRowModel().rows.map((row) => (
          <tr key={row.id} data-testid="calibration-row">
            <td className="question">{row.original.question}</td>
            <td>{bandLabel(row.original.band)}</td>
            <td className="num">{countLabel(row.original.confirmed)}</td>
            <td className="num">{countLabel(row.original.dismissed)}</td>
            <td className="num">{reviewedLabel(row.original.reviewed)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function numeric(id: string): boolean {
  return id === "confirmed" || id === "dismissed" || id === "reviewed";
}

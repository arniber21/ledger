import { describe, expect, test } from "bun:test";
import { groupCalibration, isSeedQuestion, questionLabel, type CalibrationRow } from "./calibration";

function row(question: string, band: string, reviewed = 1): CalibrationRow {
  return { question, band, confirmed: reviewed, dismissed: 0, reviewed };
}

describe("calibration grouping", () => {
  test("hides test and seed questions", () => {
    expect(isSeedQuestion("cal-078c0632-7082-417f-af4d-9ea126e584de")).toBe(true);
    expect(isSeedQuestion("cal-other-082e1693-184f-49f3-96ee-83036cbce233")).toBe(true);
    expect(isSeedQuestion("same_obligation")).toBe(false);
  });

  test("names known questions and sentence-cases the rest", () => {
    expect(questionLabel("same_obligation")).toBe("Same obligation");
    expect(questionLabel("memo_instructs_payment")).toBe("Memo instructs payment");
    expect(questionLabel("new_question_id")).toBe("New question id");
  });

  test("groups bands under one question and totals reviews", () => {
    const groups = groupCalibration([
      row("same_obligation", "0.80-0.90", 3),
      row("cal-1b2c3d4e-0000-4000-8000-000000000000", "0.00-0.10"),
      row("same_obligation", "unknown", 2),
      row("memo_instructs_payment", "0.90-1.00"),
    ]);
    expect(groups.map((group) => group.label)).toEqual(["Same obligation", "Memo instructs payment"]);
    expect(groups[0]?.rows.map((item) => item.band)).toEqual(["0.80-0.90", "unknown"]);
    expect(groups[0]?.reviewed).toBe(5);
  });

  test("only seed rows leaves nothing to show", () => {
    expect(groupCalibration([row("cal-abc", "unknown")])).toEqual([]);
  });
});

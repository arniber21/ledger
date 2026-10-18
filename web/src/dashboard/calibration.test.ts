import { describe, expect, test } from "bun:test";
import {
  bandLabel,
  parseCalibration,
  reviewedLabel,
  thresholdLabel,
} from "./calibration";

describe("calibration table", () => {
  test("formats score bands and keeps unknown inconclusive", () => {
    expect(bandLabel("0.80-0.90")).toBe("0.80–0.90");
    expect(bandLabel("0.90-1.00")).toBe("0.90–1.00");
    expect(bandLabel("unknown")).toBe("Unknown · inconclusive");
    expect(bandLabel("unknown").toLowerCase()).not.toContain("pass");
    expect(bandLabel("unknown").toLowerCase()).not.toContain("fraud");
  });

  test("marks a small sample and still shows the count", () => {
    expect(reviewedLabel(0)).toBe("0 · small sample");
    expect(reviewedLabel(9)).toBe("9 · small sample");
    expect(reviewedLabel(10)).toBe("10");
    expect(reviewedLabel(1000)).toBe("1,000");
  });

  test("reads the fixed threshold as 0.80", () => {
    expect(thresholdLabel(0.8)).toBe("0.80");
    expect(thresholdLabel(undefined)).toBe("0.80");
  });

  test("parses stored rows and leaves an empty table empty", () => {
    expect(parseCalibration({ threshold: 0.8, rows: [] })).toEqual({
      thresholdLabel: "0.80",
      rows: [],
    });
    expect(
      parseCalibration({
        threshold: 0.8,
        rows: [
          { question: "same_obligation", band: "0.80-0.90", confirmed: 2, dismissed: 1, reviewed: 3 },
          { question: "", band: "unknown", confirmed: 1, dismissed: 0, reviewed: 1 },
          { band: "0.90-1.00", confirmed: 1, dismissed: 0, reviewed: 1 },
        ],
      }),
    ).toEqual({
      thresholdLabel: "0.80",
      rows: [
        { question: "same_obligation", band: "0.80-0.90", confirmed: 2, dismissed: 1, reviewed: 3 },
      ],
    });
  });

  test("rejects a payload that is not a table", () => {
    expect(parseCalibration(null)).toBeNull();
    expect(parseCalibration({ rows: "none" })).toBeNull();
  });
});

import { describe, expect, test } from "bun:test";
import { faultFailure, parseFaultBody } from "./faults";

describe("fault control", () => {
  test("reads enabled only when it is a boolean", () => {
    expect(parseFaultBody({ enabled: true })).toBe(true);
    expect(parseFaultBody({ enabled: false })).toBe(false);
    expect(parseFaultBody({ enabled: "yes" })).toBeNull();
    expect(parseFaultBody(null)).toBeNull();
  });
  test("failure line names the fault", () => {
    expect(faultFailure("double-pay", "The desk did not answer.")).toContain("double-pay");
  });
});

import { describe, expect, test } from "bun:test";
import { parseCombo } from "./hotkeys";

describe("parseCombo", () => {
  test("sequence", () => expect(parseCombo("g i").map((s) => s.key)).toEqual(["g", "i"]));
  test("mod", () => expect(parseCombo("mod+a")[0]).toMatchObject({ key: "a", mod: true }));
  test("typing prefix", () => expect(parseCombo("!escape")[0]?.key).toBe("escape"));
});

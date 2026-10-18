import { describe, expect, test } from "bun:test";
import { parseCombo } from "./hotkeys";

describe("parseCombo", () => {
  test("sequence", () => expect(parseCombo("g i").map((s) => s.key)).toEqual(["g", "i"]));
  test("mod", () => expect(parseCombo("mod+a")[0]).toMatchObject({ key: "a", mod: true }));
  test("typing prefix", () => expect(parseCombo("!escape")[0]?.key).toBe("escape"));
});

import { stepMatches } from "./hotkeys";

function key(init: Partial<KeyboardEvent> & { key: string }): KeyboardEvent {
  return { metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...init } as KeyboardEvent;
}

describe("stepMatches", () => {
  const [j] = parseCombo("j");
  const [shiftJ] = parseCombo("shift+j");
  const [modA] = parseCombo("mod+a");
  test("plain j does not match shifted J", () => {
    expect(stepMatches(j!, key({ key: "j" }))).toBe(true);
    expect(stepMatches(j!, key({ key: "J", shiftKey: true }))).toBe(false);
  });
  test("shift+j matches shifted J only", () => {
    expect(stepMatches(shiftJ!, key({ key: "J", shiftKey: true }))).toBe(true);
    expect(stepMatches(shiftJ!, key({ key: "j" }))).toBe(false);
  });
  test("mod+a matches cmd or ctrl", () => {
    expect(stepMatches(modA!, key({ key: "a", metaKey: true }))).toBe(true);
    expect(stepMatches(modA!, key({ key: "a", ctrlKey: true }))).toBe(true);
    expect(stepMatches(modA!, key({ key: "a" }))).toBe(false);
  });
  test("question mark matches with shift held", () => {
    const [q] = parseCombo("?");
    expect(stepMatches(q!, key({ key: "?", shiftKey: true }))).toBe(true);
  });
});

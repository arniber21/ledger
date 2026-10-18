import { describe, expect, test } from "bun:test";
import { pushRecent } from "./palette";

describe("recent palette items", () => {
  test("moves a repeat to the front and caps the list", () => {
    expect(pushRecent(["a", "b", "c"], "b")).toEqual(["b", "a", "c"]);
    expect(pushRecent(["1", "2", "3", "4", "5"], "6")).toEqual(["6", "1", "2", "3", "4"]);
  });
});

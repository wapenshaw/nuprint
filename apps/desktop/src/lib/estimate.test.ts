import { describe, expect, it } from "vitest";
import { clampInt, formatMm, paperEstimateMm } from "./estimate";

describe("paperEstimateMm", () => {
  it("applies the extra feed once, after the final copy", () => {
    expect(paperEstimateMm(80, 1, 12)).toBe(92);
    expect(paperEstimateMm(80, 3, 12)).toBe(252); // 3 x 80 + 12, not 3 x (80 + 12)
  });

  it("supports zero feed", () => {
    expect(paperEstimateMm(50.5, 2, 0)).toBe(101);
  });

  it("treats invalid copies as one copy and negative feed as none", () => {
    expect(paperEstimateMm(10, 0, 5)).toBe(15);
    expect(paperEstimateMm(10, Number.NaN, -3)).toBe(10);
  });
});

describe("clampInt", () => {
  it("clamps and rounds to the bounds", () => {
    expect(clampInt("99", 0, 12, 12)).toBe(12);
    expect(clampInt(-4, 0, 12, 12)).toBe(0);
    expect(clampInt("5.6", 0, 12, 12)).toBe(6);
  });
  it("falls back for non-numeric input", () => {
    expect(clampInt("", 0, 12, 7)).toBe(7);
    expect(clampInt("abc", 1, 20, 1)).toBe(1);
  });
});

describe("formatMm", () => {
  it("shows at most one decimal", () => {
    expect(formatMm(92)).toBe("92 mm");
    expect(formatMm(92.04)).toBe("92 mm");
    expect(formatMm(92.46)).toBe("92.5 mm");
  });
});

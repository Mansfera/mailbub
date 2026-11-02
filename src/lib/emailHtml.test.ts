import { describe, expect, test } from "bun:test";
import { estimateSize } from "./emailHtml";

const flat = { x: 0, y: 0 };

describe("estimateSize", () => {
  test("wraps long runs of text at the available width", () => {
    const hints = { lines: [10, 250], images: [], inset: flat };
    // 70 characters per line: one line, then four.
    expect(estimateSize(hints, 490).height).toBe(5 * 21);
    expect(estimateSize(hints, 980).height).toBe(3 * 21);
  });

  test("counts blank lines and the page's padding", () => {
    const hints = { lines: [5, 0, 5], images: [], inset: { x: 16, y: 14 } };
    expect(estimateSize(hints, 680).height).toBe(3 * 21 + 28);
  });

  test("scales images down to the frame, never up", () => {
    const image = (width: number, height: number) => ({
      lines: [],
      images: [{ width, height }],
      inset: flat,
    });
    expect(estimateSize(image(1000, 500), 500).height).toBe(250);
    expect(estimateSize(image(200, 100), 500).height).toBe(100);
  });

  test("an empty message still gets one line", () => {
    expect(estimateSize({ lines: [], images: [], inset: flat }, 500).height).toBe(21);
  });

  test("the width follows the longest line", () => {
    expect(estimateSize({ lines: [3, 10], images: [], inset: flat }, 500).width).toBe(70);
  });
});

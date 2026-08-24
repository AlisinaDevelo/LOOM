import { describe, expect, it } from "vitest";

import { compactPath, projectImageRegion } from "./evidence";

describe("compactPath", () => {
  it("keeps short paths intact and bounds long paths", () => {
    expect(compactPath("/tmp/notes.md")).toBe("/tmp/notes.md");
    expect(compactPath(`/Users/name/${"nested/".repeat(20)}notes.md`, 44)).toHaveLength(44);
    expect(compactPath("a-very-long-path", 8)).toHaveLength(8);
    expect(compactPath("long", 1)).toBe("…");
    expect(compactPath("long", 0)).toBe("");
  });
});

describe("projectImageRegion", () => {
  const anchor = {
    x: 100,
    y: 50,
    width: 200,
    height: 100,
    image_width: 1200,
    image_height: 600,
  };

  it("keeps an oriented region stable at 1x", () => {
    const projection = projectImageRegion(anchor);
    expect(projection.leftPercent).toBeCloseTo(8.3333, 3);
    expect(projection.topPercent).toBeCloseTo(8.3333, 3);
    expect(projection.widthPercent).toBeCloseTo(16.6667, 3);
    expect(projection.heightPercent).toBeCloseTo(16.6667, 3);
    expect(projection).toMatchObject({ canvasWidth: 1200, canvasHeight: 600, rotation: 0, scale: 1 });
  });

  it("rotates the rectangle and swaps the canvas dimensions", () => {
    const projection = projectImageRegion(anchor, 1.5, 90, 2);
    expect(projection.leftPercent).toBeCloseTo(75, 3);
    expect(projection.topPercent).toBeCloseTo(8.3333, 3);
    expect(projection.widthPercent).toBeCloseTo(16.6667, 3);
    expect(projection.heightPercent).toBeCloseTo(16.6667, 3);
    expect(projection).toMatchObject({ canvasWidth: 1800, canvasHeight: 3600, rotation: 90, scale: 3 });
  });

  it("clamps invalid zoom, scale, and out-of-bounds anchors", () => {
    expect(projectImageRegion({ ...anchor, x: 2_000, y: -20, width: 400, height: 700 }, 10, 450, 0.1)).toMatchObject({
      leftPercent: 0,
      topPercent: 100,
      widthPercent: 100,
      heightPercent: 0,
      rotation: 90,
      scale: 2,
    });
  });
});

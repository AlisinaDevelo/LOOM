import { describe, expect, it } from "vitest";

import { compactPath } from "./evidence";

describe("compactPath", () => {
  it("keeps short paths intact and bounds long paths", () => {
    expect(compactPath("/tmp/notes.md")).toBe("/tmp/notes.md");
    expect(compactPath(`/Users/name/${"nested/".repeat(20)}notes.md`, 44)).toHaveLength(44);
    expect(compactPath("a-very-long-path", 8)).toHaveLength(8);
    expect(compactPath("long", 1)).toBe("…");
    expect(compactPath("long", 0)).toBe("");
  });
});

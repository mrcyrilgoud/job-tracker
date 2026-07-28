import { describe, expect, it } from "vitest";

import { isDesktopShell } from "@/lib/tauri";

describe("isDesktopShell", () => {
  it("is false outside the Tauri window", () => {
    expect(isDesktopShell()).toBe(false);
  });
});

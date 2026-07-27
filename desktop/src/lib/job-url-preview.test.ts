import { describe, expect, it } from "vitest";

import { applyJobUrlPreview } from "./job-url-preview";

describe("applyJobUrlPreview", () => {
  it("fills both blank fields from a complete preview", () => {
    expect(
      applyJobUrlPreview(
        { title: "", companyName: "" },
        { title: "Software Engineer", companyName: "Acme" },
      ),
    ).toEqual({ title: "Software Engineer", companyName: "Acme" });
  });

  it("preserves non-empty manually entered values", () => {
    expect(
      applyJobUrlPreview(
        { title: "My title", companyName: "My company" },
        { title: "Detected title", companyName: "Detected company" },
      ),
    ).toEqual({ title: "My title", companyName: "My company" });
  });

  it("fills only the available blank field from a partial preview", () => {
    expect(
      applyJobUrlPreview(
        { title: "", companyName: "" },
        { title: "Product Designer", companyName: null },
      ),
    ).toEqual({ title: "Product Designer", companyName: "" });
  });

  it("leaves current values unchanged for an empty preview", () => {
    expect(
      applyJobUrlPreview(
        { title: "", companyName: "Existing company" },
        { title: null, companyName: null },
      ),
    ).toEqual({ title: "", companyName: "Existing company" });
  });
});

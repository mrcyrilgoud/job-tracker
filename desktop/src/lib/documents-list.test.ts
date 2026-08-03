import { describe, expect, it, vi } from "vitest";

import type { DocumentListItem } from "./schema";

describe("documents list DTO contract", () => {
  it("uses flattened document id for Open actions", async () => {
    const openDocument = vi.fn();
    const documents: DocumentListItem[] = [
      {
        id: "doc-1",
        originalFilename: "resume.pdf",
        storedFilename: "doc-1.pdf",
        mimeType: "application/pdf",
        checksum: "abc",
        sizeBytes: 10,
        importedAt: "2026-01-01T00:00:00Z",
        usedBy: ["Acme — Engineer"],
        kinds: ["resume"],
      },
    ];

    // Simulate DocumentsPage Open handler contract.
    const doc = documents[0];
    await openDocument(doc.id);

    expect(openDocument).toHaveBeenCalledWith("doc-1");
    expect(doc.originalFilename).toBe("resume.pdf");
    expect(doc.usedBy).toEqual(["Acme — Engineer"]);
    expect(doc.kinds).toEqual(["resume"]);
  });
});

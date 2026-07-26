import path from "node:path";
import { describe, expect, it } from "vitest";
import fs from "node:fs";

import { getDocumentsDir } from "@/lib/db";
import { importDocument, attachDocumentToJob } from "@/lib/documents/store";
import { createJobFromUrl, listJobs, updateJob } from "@/lib/jobs/service";
import { safeFetch } from "@/lib/jobs/safe-fetch";

describe("job CRUD", () => {
  it("creates, updates, and lists a job with timeline status changes", async () => {
    const unique = Date.now();
    const { job } = await createJobFromUrl({
      url: `https://example.com/jobs/role-${unique}`,
      title: `Role ${unique}`,
      companyName: `Company ${unique}`,
      status: "wishlist",
    });

    expect(job.title).toContain(String(unique));

    const updated = updateJob(job.id, { status: "applied" });
    expect(updated?.job.status).toBe("applied");
    expect(updated?.events.some((event) => event.type === "status_changed")).toBe(true);

    const listed = listJobs({ companyId: job.companyId });
    expect(listed.some((row) => row.job.id === job.id)).toBe(true);
  });

  it("rejects duplicate canonical URLs", async () => {
    const unique = Date.now() + 1;
    const url = `https://example.com/jobs/dup-${unique}`;
    await createJobFromUrl({
      url,
      title: "Dup A",
      companyName: `DupCo ${unique}`,
    });
    await expect(
      createJobFromUrl({
        url: `${url}/?utm_source=x`,
        title: "Dup B",
        companyName: `DupCo ${unique}`,
      }),
    ).rejects.toThrow(/already tracked/);
  });
});

describe("documents", () => {
  it("imports and attaches an immutable document copy", async () => {
    const unique = Date.now() + 2;
    const { job } = await createJobFromUrl({
      url: `https://example.com/jobs/docs-${unique}`,
      title: `Docs Role ${unique}`,
      companyName: `DocsCo ${unique}`,
    });

    const buffer = Buffer.from(`resume content ${unique}`);
    const document = importDocument({
      originalFilename: `resume-${unique}.txt`,
      mimeType: "text/plain",
      buffer,
    });

    const filePath = path.join(getDocumentsDir(), document.storedFilename);
    expect(fs.existsSync(filePath)).toBe(true);

    const again = importDocument({
      originalFilename: `resume-copy-${unique}.txt`,
      mimeType: "text/plain",
      buffer,
    });
    expect(again.id).toBe(document.id);

    const attachment = attachDocumentToJob({
      jobId: job.id,
      documentId: document.id,
      kind: "resume",
    });
    expect(attachment.kind).toBe("resume");
  });
});

describe("safeFetch SSRF guards", () => {
  it("rejects loopback hosts", async () => {
    const result = await safeFetch("http://127.0.0.1/");
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/private|local/i);
  });

  it("rejects non-http protocols", async () => {
    const result = await safeFetch("file:///etc/passwd");
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/HTTP/);
  });
});

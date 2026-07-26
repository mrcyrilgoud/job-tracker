import { describe, expect, it } from "vitest";

import { parseAtsJobsFromJson } from "@/lib/ats";
import { classifyEmail } from "@/lib/gmail/classify";
import { looksLikeClosedPosting } from "@/lib/jobs/safe-fetch";
import { normalizeCanonicalUrl } from "@/lib/utils";
import { validateDocumentUpload } from "@/lib/documents/store";

describe("normalizeCanonicalUrl", () => {
  it("strips tracking params and trailing slash", () => {
    expect(
      normalizeCanonicalUrl(
        "https://Jobs.Example.com/role/design/?utm_source=x&gh_src=y#frag",
      ),
    ).toBe("https://jobs.example.com/role/design");
  });
});

describe("looksLikeClosedPosting", () => {
  it("treats 404/410 as inactive", () => {
    expect(looksLikeClosedPosting("", 404)).toBe(true);
    expect(looksLikeClosedPosting("", 410)).toBe(true);
  });

  it("detects closed copy", () => {
    expect(
      looksLikeClosedPosting("<html>This job is closed. Thanks.</html>", 200),
    ).toBe(true);
  });
});

describe("document validation", () => {
  it("accepts pdf", () => {
    expect(() =>
      validateDocumentUpload("resume.pdf", "application/pdf", 1024),
    ).not.toThrow();
  });

  it("rejects oversize files", () => {
    expect(() =>
      validateDocumentUpload("resume.pdf", "application/pdf", 20 * 1024 * 1024),
    ).toThrow(/10MB/);
  });

  it("rejects mismatched extension", () => {
    expect(() =>
      validateDocumentUpload("resume.txt", "application/pdf", 1024),
    ).toThrow(/extension/);
  });
});

describe("ATS fixtures", () => {
  it("parses greenhouse", () => {
    const jobs = parseAtsJobsFromJson(
      "greenhouse",
      JSON.stringify({
        jobs: [
          {
            id: 1,
            title: "Designer",
            absolute_url: "https://boards.greenhouse.io/acme/jobs/1",
            location: { name: "Remote" },
          },
        ],
      }),
    );
    expect(jobs).toEqual([
      {
        externalId: "1",
        title: "Designer",
        url: "https://boards.greenhouse.io/acme/jobs/1",
        location: "Remote",
      },
    ]);
  });

  it("parses lever", () => {
    const jobs = parseAtsJobsFromJson(
      "lever",
      JSON.stringify([
        {
          id: "abc",
          text: "Engineer",
          hostedUrl: "https://jobs.lever.co/acme/abc",
          categories: { location: "NYC" },
        },
      ]),
    );
    expect(jobs[0]?.externalId).toBe("abc");
  });

  it("parses ashby", () => {
    const jobs = parseAtsJobsFromJson(
      "ashby",
      JSON.stringify({
        jobs: [
          {
            id: "ash-1",
            title: "PM",
            jobUrl: "https://jobs.ashbyhq.com/acme/ash-1",
            location: "SF",
          },
        ],
      }),
    );
    expect(jobs[0]?.title).toBe("PM");
  });

  it("rejects malformed greenhouse", () => {
    expect(() => parseAtsJobsFromJson("greenhouse", JSON.stringify({}))).toThrow(
      /Unexpected Greenhouse/,
    );
  });
});

describe("gmail classifyEmail", () => {
  it("marks high confidence when company, title, and signal match", () => {
    expect(
      classifyEmail({
        subject: "Acme interview for Senior Designer",
        snippet: "Thanks for your application",
        fromAddress: "recruiting@acme.com",
        companyName: "Acme",
        jobTitle: "Senior Designer",
      }),
    ).toBe("high");
  });

  it("marks low confidence for company-only newsletter", () => {
    expect(
      classifyEmail({
        subject: "Acme monthly newsletter",
        snippet: "Product updates this month",
        fromAddress: "hello@acme.com",
        companyName: "Acme",
        jobTitle: "Senior Designer",
      }),
    ).toBe("low");
  });
});

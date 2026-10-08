// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { JobListItem } from "@/lib/schema";

import { NewRolesPanel } from "./NewRolesPanel";

function role(id: string): JobListItem {
  return {
    companyName: "Acme",
    job: {
      id,
      companyId: "company-1",
      title: `Engineer ${id}`,
      url: `https://example.com/jobs/${id}`,
      canonicalUrl: `https://example.com/jobs/${id}`,
      sourceExternalId: id,
      status: "wishlist",
      appliedAt: null,
      postingState: "active",
      lastCheckedAt: null,
      lastCheckResult: null,
      source: "greenhouse",
      notes: null,
      description: null,
      location: "San Francisco",
      isNewFromWatch: true,
      watchDisposition: "new",
      missingFromSyncCount: 0,
      isFavorite: false,
      appeal: null,
      salaryMin: null,
      salaryMax: null,
      createdAt: "2026-10-01T00:00:00.000Z",
      updatedAt: "2026-10-01T00:00:00.000Z",
    },
  };
}

describe("NewRolesPanel company preview", () => {
  let root: Root | null = null;
  let host: HTMLDivElement | null = null;

  afterEach(() => {
    if (root) act(() => root?.unmount());
    host?.remove();
    root = null;
    host = null;
    vi.unstubAllGlobals();
  });

  it("keeps the six-role collapsed preview and reveals the remainder on request", async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    const roles = Array.from({ length: 7 }, (_, index) => role(`role-${index + 1}`));

    await act(async () => {
      root?.render(
        <NewRolesPanel roles={roles} onTriage={() => {}} isPending={() => false} />,
      );
    });

    expect(host.querySelectorAll("li")).toHaveLength(6);
    expect([...host.querySelectorAll("button")].some((button) => button.textContent?.trim() === "Show 1 more")).toBe(true);

    await act(async () => {
      [...host!.querySelectorAll("button")]
        .find((button) => button.textContent?.trim() === "Show 1 more")
        ?.click();
    });

    expect(host.querySelectorAll("li")).toHaveLength(7);
  });
});

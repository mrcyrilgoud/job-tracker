import type { JobSource, JobStatus, PostingState } from "@/lib/schema";

export type Tone = "violet" | "blue" | "amber" | "green" | "stone";

export const toneClasses: Record<Tone, string> = {
  violet: "bg-[var(--violet-soft)] text-[var(--violet-ink)]",
  blue: "bg-[var(--blue-soft)] text-[var(--blue-ink)]",
  amber: "bg-[var(--amber-soft)] text-[var(--amber-ink)]",
  green: "bg-[var(--green-soft)] text-[var(--green-ink)]",
  stone: "bg-[var(--stone-soft)] text-[var(--stone-ink)]",
};

export type Presentation = {
  label: string;
  tone: Tone;
};

export function jobStatusPresentation(status: JobStatus): Presentation {
  switch (status) {
    case "wishlist":
      return { label: "Wishlist", tone: "violet" };
    case "applied":
      return { label: "Applied", tone: "blue" };
    case "interviewing":
      return { label: "Interviewing", tone: "amber" };
    case "offer":
      return { label: "Offer", tone: "green" };
    case "rejected":
      return { label: "Rejected", tone: "stone" };
    case "withdrawn":
      return { label: "Withdrawn", tone: "stone" };
    case "closed":
      return { label: "Closed", tone: "stone" };
    default: {
      const _exhaustive: never = status;
      return _exhaustive;
    }
  }
}

export function postingStatePresentation(state: PostingState): Presentation {
  switch (state) {
    case "active":
      return { label: "Open", tone: "green" };
    case "inactive":
      return { label: "Closed", tone: "stone" };
    case "unknown":
      return { label: "Not checked yet", tone: "stone" };
    default: {
      const _exhaustive: never = state;
      return _exhaustive;
    }
  }
}

export function jobSourceLabel(source: JobSource): string | null {
  switch (source) {
    case "manual":
      return null;
    case "greenhouse":
      return "Found via Greenhouse watch";
    case "lever":
      return "Found via Lever watch";
    case "ashby":
      return "Found via Ashby watch";
    case "careers_page":
      return "Found on a careers page";
    default: {
      const _exhaustive: never = source;
      return _exhaustive;
    }
  }
}

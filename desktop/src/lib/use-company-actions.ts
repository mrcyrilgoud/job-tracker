import { useCallback, useState } from "react";

import type { TriageAction } from "@/components/companies/NewRolesList";
import { api } from "@/lib/api";
import { roleCountLabel, syncErrorNote } from "@/lib/companies-ui";
import { usePendingActions } from "@/lib/use-pending-actions";

/**
 * Every action a company card can take, shared by the Companies list and the
 * single-company route so the two surfaces cannot drift apart.
 *
 * Keys are `kind:id` so each control tracks its own progress and its own
 * result message — previously one page-wide `busy` flag disabled every button
 * and every outcome was reported in a banner at the top of the page.
 */
export function useCompanyActions(onChanged: () => void) {
  const { isPending, run, feedback } = usePendingActions();
  const [openRoles, setOpenRoles] = useState<ReadonlySet<string>>(new Set());

  const toggleRoles = useCallback((companyId: string, force?: boolean) => {
    setOpenRoles((current) => {
      const next = new Set(current);
      if (force ?? !next.has(companyId)) {
        next.add(companyId);
      } else {
        next.delete(companyId);
      }
      return next;
    });
  }, []);

  const syncWatch = useCallback(
    (watchId: string, companyId: string, provider: string) => {
      void run(
        `sync:${watchId}`,
        () => api.syncWatch(watchId),
        (data) => {
          if (!data.ok) {
            return { tone: "negative", text: syncErrorNote(data.error ?? "", provider) };
          }
          if (data.created > 0) {
            // Open the drawer so what this run found is one glance away rather
            // than something the user has to go hunting for on another tab.
            toggleRoles(companyId, true);
            return { tone: "positive", text: `Found ${roleCountLabel(data.created)} below.` };
          }
          return { tone: "positive", text: "No new roles this time." };
        },
        "Couldn't check that board.",
      ).then(() => onChanged());
    },
    [onChanged, run, toggleRoles],
  );

  const removeWatch = useCallback(
    (watchId: string) => {
      void run(`remove:${watchId}`, () => api.deleteWatch(watchId), () => null).then(() =>
        onChanged(),
      );
    },
    [onChanged, run],
  );

  const checkCareers = useCallback(
    (companyId: string) => {
      void run(
        `careers:${companyId}`,
        () => api.checkCareers(companyId),
        (data) =>
          data.changed
            ? { tone: "positive", text: "The careers page changed — see the note above." }
            : { tone: "positive", text: "No changes since we last looked." },
        "Couldn't reach that careers page.",
      ).then(() => onChanged());
    },
    [onChanged, run],
  );

  const dismissReview = useCallback(
    (reviewId: string) => {
      void run(`review:${reviewId}`, () => api.dismissReview(reviewId), () => null).then(() =>
        onChanged(),
      );
    },
    [onChanged, run],
  );

  const triageRole = useCallback(
    (jobId: string, action: TriageAction) => {
      void run(
        `triage:${jobId}`,
        () => (action === "save" ? api.approveWatchJob(jobId) : api.dismissWatchJob(jobId)),
        () => null,
      ).then(() => onChanged());
    },
    [onChanged, run],
  );

  const saveOpenRole = useCallback(
    (jobId: string) => {
      void run(`save-open:${jobId}`, () => api.saveOpenWatchJob(jobId), () => null).then(() =>
        onChanged(),
      );
    },
    [onChanged, run],
  );

  const resetDismissedRole = useCallback(
    (jobId: string) => {
      void run(`reset:${jobId}`, () => api.resetDismissedWatchJob(jobId), () => null).then(() =>
        onChanged(),
      );
    },
    [onChanged, run],
  );

  return {
    isPending,
    feedback,
    openRoles,
    toggleRoles,
    syncWatch,
    removeWatch,
    checkCareers,
    dismissReview,
    triageRole,
    saveOpenRole,
    resetDismissedRole,
  };
}

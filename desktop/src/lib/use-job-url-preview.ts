import { useCallback, useRef, useState } from "react";

import { api, type ConfirmedJobDiscovery, type JobUrlPreview } from "@/lib/api";
import {
  applyJobUrlPreview,
  confirmJobUrlPreview,
  resetJobUrlDiscovery,
} from "@/lib/job-url-preview";

function isValidPostingUrl(value: string) {
  try {
    const parsed = new URL(value);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

function normalizeUrl(value: string) {
  return value.trim();
}

/**
 * Shared URL preview/detection with stale-response guards.
 * Accepts a result only when it matches the newest request and current URL.
 */
export function useJobUrlPreview(args: {
  url: string;
  title: string;
  companyName: string;
  setTitle: (value: string | ((current: string) => string)) => void;
  setCompanyName: (value: string | ((current: string) => string)) => void;
}) {
  const { url, setTitle, setCompanyName } = args;
  const [preview, setPreview] = useState<JobUrlPreview | null>(null);
  const [confirmedDiscovery, setConfirmedDiscovery] =
    useState<ConfirmedJobDiscovery | null>(null);
  const [autofillError, setAutofillError] = useState<string | null>(null);
  const [autofillStatus, setAutofillStatus] = useState<string | null>(null);
  const [isAutofilling, setIsAutofilling] = useState(false);
  const requestIdRef = useRef(0);
  const urlRef = useRef(url);
  const titleRef = useRef(args.title);
  const companyRef = useRef(args.companyName);
  urlRef.current = url;
  titleRef.current = args.title;
  companyRef.current = args.companyName;

  const clearDiscoveryForUrlChange = useCallback(() => {
    requestIdRef.current += 1;
    setPreview(null);
    setConfirmedDiscovery(null);
    setAutofillError(null);
    setAutofillStatus(null);
  }, []);

  const onAutofill = useCallback(async () => {
    const postingUrl = normalizeUrl(urlRef.current);
    const requestId = ++requestIdRef.current;
    setAutofillError(null);
    setAutofillStatus(null);

    if (!isValidPostingUrl(postingUrl)) {
      setAutofillError("Enter a valid http or https posting link.");
      return;
    }

    setIsAutofilling(true);
    try {
      const nextPreview = await api.previewJobUrl(postingUrl);
      if (
        requestId !== requestIdRef.current ||
        normalizeUrl(urlRef.current) !== postingUrl
      ) {
        return;
      }
      setPreview(nextPreview);
      setConfirmedDiscovery(resetJobUrlDiscovery().confirmedDiscovery);
      setTitle((current) =>
        applyJobUrlPreview(
          { title: current, companyName: companyRef.current },
          nextPreview,
        ).title,
      );
      setCompanyName((current) =>
        applyJobUrlPreview(
          { title: titleRef.current, companyName: current },
          nextPreview,
        ).companyName,
      );

      if (nextPreview.title && nextPreview.companyName) {
        setAutofillStatus("Found the role title and company.");
      } else if (nextPreview.title) {
        setAutofillStatus("Found the role title. Company could not be found.");
      } else if (nextPreview.companyName) {
        setAutofillStatus("Found the company. Role title could not be found.");
      } else {
        setAutofillStatus("No title or company was found. Enter the details manually.");
      }
    } catch {
      if (
        requestId !== requestIdRef.current ||
        normalizeUrl(urlRef.current) !== postingUrl
      ) {
        return;
      }
      setAutofillError("Could not autofill details. Check the link and try again.");
    } finally {
      if (requestId === requestIdRef.current) {
        setIsAutofilling(false);
      }
    }
  }, [setCompanyName, setTitle]);

  return {
    preview,
    confirmedDiscovery,
    setConfirmedDiscovery,
    autofillError,
    autofillStatus,
    isAutofilling,
    onAutofill,
    clearDiscoveryForUrlChange,
    confirmBoard: () => {
      if (!preview) return;
      setConfirmedDiscovery(confirmJobUrlPreview(preview));
    },
    isValidPostingUrl,
  };
}

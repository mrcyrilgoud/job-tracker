import { useEffect, useId, useRef, useState } from "react";
import { useLocation } from "react-router-dom";
import { useRunMonitor } from "@/lib/RunMonitorContext";
import { isRunBusy, runStatusLabel } from "@/lib/run-state";
import { RunControls } from "./RunControls";

/** A compact side disclosure; opening it never changes page or header geometry. */
export function ChecksTab({ panelVisible, onShowRun }: {
  panelVisible: boolean;
  onShowRun: () => void;
}) {
  const [open, setOpen] = useState(false);
  const { pathname } = useLocation();
  const { state } = useRunMonitor();
  const container = useRef<HTMLDivElement>(null);
  const toggle = useRef<HTMLButtonElement>(null);
  const panelId = useId();
  const busy = isRunBusy(state);
  const status = state.displayed ? runStatusLabel(state.displayed.runStatus) : null;

  useEffect(() => { setOpen(false); }, [pathname]);
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !container.current?.contains(event.target)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setOpen(false);
      toggle.current?.focus();
    };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("keydown", escape);
    };
  }, [open]);

  return (
    <div className="checks-tab" ref={container}>
      <button type="button" className="btn btn-secondary btn-sm checks-toggle" ref={toggle}
        aria-expanded={open} aria-controls={panelId} onClick={() => setOpen(!open)}
        title={status ? `Checks: ${status}` : "Open checks"}>
        Checks
        {busy ? <span className="checks-busy-dot" aria-label="Run in progress" /> : null}
      </button>
      <section id={panelId} className="checks-panel" aria-label="Checks" hidden={!open}>
        <h2 className="mb-3 text-base font-semibold">Checks</h2>
        <RunControls panelVisible={panelVisible} onShowRun={() => {
          setOpen(false);
          toggle.current?.focus();
          onShowRun();
        }} />
      </section>
      {!open && state.notice ? (
        <p className="checks-notice text-xs text-[var(--amber-ink)]" role="status">
          {state.notice.message}
        </p>
      ) : null}
    </div>
  );
}

import { createPortal } from "react-dom";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { ChevronDownIcon, CheckIcon, statusIcons } from "@/components/icons";
import { jobStatuses, type JobStatus } from "@/lib/schema";
import { jobStatusPresentation, toneClasses } from "@/lib/ui";

type ChangeableStatus = Exclude<JobStatus, "archived">;
const STATUS_OPTIONS: ChangeableStatus[] = jobStatuses.filter(
  (status): status is ChangeableStatus => status !== "archived",
);

type StatusQuickChangeProps = {
  currentStatus: ChangeableStatus;
  onStatusChange: (nextStatus: ChangeableStatus) => void | Promise<void>;
  pending?: boolean;
  error?: string | null;
  onRetry?: () => void;
  size?: "sm" | "md";
  className?: string;
};

export function StatusQuickChange({
  currentStatus,
  onStatusChange,
  pending = false,
  error = null,
  onRetry,
  size = "md",
  className = "",
}: StatusQuickChangeProps) {
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(() => Math.max(0, STATUS_OPTIONS.indexOf(currentStatus)));
  const [position, setPosition] = useState<{ top: number; left: number; maxHeight: number } | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);

  const currentInfo = jobStatusPresentation(currentStatus);
  const CurrentIcon = statusIcons[currentStatus];

  const closeMenu = useCallback((restoreFocus = true) => {
    setOpen(false);
    if (restoreFocus) {
      window.requestAnimationFrame(() => triggerRef.current?.focus());
    }
  }, []);

  const updatePosition = useCallback(() => {
    const trigger = triggerRef.current;
    const menu = menuRef.current;
    if (!trigger || !menu) return;
    const rect = trigger.getBoundingClientRect();
    const menuRect = menu.getBoundingClientRect();
    const padding = 8;
    const gap = 6;
    const roomBelow = window.innerHeight - rect.bottom - padding;
    const roomAbove = rect.top - padding;
    const shouldFlip = roomBelow < Math.min(menuRect.height, 360) && roomAbove > roomBelow;
    const maxHeight = Math.max(160, Math.min(448, (shouldFlip ? roomAbove : roomBelow)));
    const top = shouldFlip
      ? Math.max(padding, rect.top - Math.min(menuRect.height, maxHeight) - gap)
      : Math.min(window.innerHeight - padding - Math.min(menuRect.height, maxHeight), rect.bottom + gap);
    const left = Math.min(
      Math.max(padding, rect.right - Math.max(menuRect.width, 220)),
      Math.max(padding, window.innerWidth - Math.max(menuRect.width, 220) - padding),
    );
    setPosition({ top, left, maxHeight });
  }, []);

  useLayoutEffect(() => {
    if (!open) return;
    updatePosition();
    const onViewportChange = () => updatePosition();
    window.addEventListener("resize", onViewportChange);
    window.addEventListener("scroll", onViewportChange, true);
    return () => {
      window.removeEventListener("resize", onViewportChange);
      window.removeEventListener("scroll", onViewportChange, true);
    };
  }, [open, updatePosition]);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (triggerRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      closeMenu(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open, closeMenu]);

  useEffect(() => {
    if (!open) return;
    const nextIndex = Math.max(0, STATUS_OPTIONS.indexOf(currentStatus));
    setActiveIndex(nextIndex);
    window.requestAnimationFrame(() => itemRefs.current[nextIndex]?.focus());
  }, [open, currentStatus]);

  const selectStatus = (nextStatus: ChangeableStatus) => {
    closeMenu();
    if (nextStatus !== currentStatus) void onStatusChange(nextStatus);
  };

  const toggleMenu = () => {
    if (open) {
      closeMenu();
      return;
    }
    const rect = triggerRef.current?.getBoundingClientRect();
    if (rect) {
      setPosition({ top: rect.bottom + 6, left: Math.max(8, rect.right - 220), maxHeight: 448 });
    }
    setOpen(true);
  };

  const moveActive = (direction: 1 | -1) => {
    setActiveIndex((current) => {
      const next = (current + direction + STATUS_OPTIONS.length) % STATUS_OPTIONS.length;
      window.requestAnimationFrame(() => itemRefs.current[next]?.focus());
      return next;
    });
  };

  const onMenuKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      moveActive(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      moveActive(-1);
    } else if (event.key === "Home") {
      event.preventDefault();
      setActiveIndex(0);
      itemRefs.current[0]?.focus();
    } else if (event.key === "End") {
      event.preventDefault();
      const last = STATUS_OPTIONS.length - 1;
      setActiveIndex(last);
      itemRefs.current[last]?.focus();
    } else if (event.key === "Escape") {
      event.preventDefault();
      closeMenu();
    }
  };

  return (
    <div className={`status-quick-change ${className}`}>
      <button
        ref={triggerRef}
        type="button"
        className={`status-quick-trigger status-quick-trigger-${size}`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-busy={pending || undefined}
        disabled={pending}
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          toggleMenu();
        }}
      >
        <span className={`status-quick-badge ${toneClasses[currentInfo.tone]}`}>
          <CurrentIcon size={size === "sm" ? 12 : 14} />
          <span>{pending ? "Saving…" : currentInfo.label}</span>
        </span>
        <ChevronDownIcon size={14} className="status-quick-chevron" />
      </button>
      {error ? (
        <span className="status-quick-error" role="alert">
          <span>{error}</span>
          {onRetry ? (
            <button type="button" onClick={onRetry} disabled={pending}>
              Retry
            </button>
          ) : null}
        </span>
      ) : null}
      {open && position
        ? createPortal(
            <div
              ref={menuRef}
              role="menu"
              aria-label={`Change status from ${currentInfo.label}`}
              aria-activedescendant={`status-option-${STATUS_OPTIONS[activeIndex]}`}
              className="status-quick-menu"
              style={{ top: position.top, left: position.left, maxHeight: position.maxHeight }}
              onKeyDown={onMenuKeyDown}
            >
              {STATUS_OPTIONS.map((status, index) => {
                const info = jobStatusPresentation(status);
                const Icon = statusIcons[status];
                const selected = status === currentStatus;
                return (
                  <button
                    key={status}
                    id={`status-option-${status}`}
                    ref={(element) => {
                      itemRefs.current[index] = element;
                    }}
                    type="button"
                    role="menuitemradio"
                    aria-checked={selected}
                    className={`status-quick-option ${selected ? "is-selected" : ""}`}
                    onMouseEnter={() => setActiveIndex(index)}
                    onClick={() => selectStatus(status)}
                  >
                    <span className={`status-quick-option-icon ${toneClasses[info.tone]}`}>
                      <Icon size={14} />
                    </span>
                    <span>{info.label}</span>
                    {selected ? <CheckIcon size={14} className="ml-auto" /> : null}
                  </button>
                );
              })}
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}

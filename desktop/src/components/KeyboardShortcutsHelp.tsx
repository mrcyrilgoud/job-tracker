import { useState, useEffect } from "react";
import { KeyboardIcon } from "@/components/icons";
import { jobStatuses, type JobStatus } from "@/lib/schema";
import { jobStatusPresentation } from "@/lib/ui";

interface KeyboardShortcutsHelpProps {
  isOpen?: boolean;
  onClose?: () => void;
}

export function KeyboardShortcutsHelp({ isOpen, onClose }: KeyboardShortcutsHelpProps) {
  const [isVisible, setIsVisible] = useState(isOpen || false);

  useEffect(() => {
    if (isOpen !== undefined) {
      setIsVisible(isOpen);
    }
  }, [isOpen]);

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      // Ctrl/Cmd + ? to toggle shortcuts help
      if ((event.ctrlKey || event.metaKey) && event.key === "?") {
        event.preventDefault();
        setIsVisible(!isVisible);
      }
      // Escape to close
      if (event.key === "Escape" && isVisible) {
        event.preventDefault();
        setIsVisible(false);
        onClose?.();
      }
    }

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [isVisible, onClose]);

  if (!isVisible) return null;

  const shortcuts = [
    { keys: ["1"], action: "Set status: Wishlist", status: "wishlist" as JobStatus },
    { keys: ["2"], action: "Set status: Applied", status: "applied" as JobStatus },
    { keys: ["3"], action: "Set status: Interviewing", status: "interviewing" as JobStatus },
    { keys: ["4"], action: "Set status: Offer", status: "offer" as JobStatus },
    { keys: ["5"], action: "Set status: Rejected", status: "rejected" as JobStatus },
    { keys: ["Ctrl", "K"], action: "Quick status change popup" },
    { keys: ["Ctrl", "S"], action: "Save changes" },
    { keys: ["Esc"], action: "Close dialogs/cancel" },
    { keys: ["Ctrl", "/"], action: "Show this help" },
    { keys: ["F"], action: "Toggle favorite" },
    { keys: ["A"], action: "Archive/restore" },
  ];

  return (
    <>
      {/* Backdrop */}
      <div
        className="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm modal-backdrop"
        onClick={() => {
          setIsVisible(false);
          onClose?.();
        }}
      />

      {/* Modal */}
      <div
        className="fixed inset-0 z-50 flex items-center justify-center p-4"
        role="dialog"
        aria-modal="true"
        aria-labelledby="keyboard-shortcuts-title"
      >
        <div
          className="modal-content w-full max-w-md rounded-2xl border border-[var(--border)] bg-[var(--surface)] p-6 shadow-[var(--shadow-xl)]"
          onClick={(e) => e.stopPropagation()}
        >
          <div className="flex items-center justify-between mb-6">
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent-ink)]">
                <KeyboardIcon size={20} />
              </div>
              <div>
                <h3 id="keyboard-shortcuts-title" className="font-display text-lg font-semibold">
                  Keyboard Shortcuts
                </h3>
                <p className="text-xs text-[var(--muted)]">
                  Speed up your workflow with these shortcuts
                </p>
              </div>
            </div>
            <button
              type="button"
              onClick={() => {
                setIsVisible(false);
                onClose?.();
              }}
              className="btn btn-ghost btn-sm -mr-2 -mt-2"
              aria-label="Close keyboard shortcuts help"
            >
              ✕
            </button>
          </div>

          <div className="space-y-3">
            {shortcuts.map((shortcut, index) => {
              const statusInfo = shortcut.status ? jobStatusPresentation(shortcut.status) : null;
              
              return (
                <div
                  key={index}
                  className="flex items-center justify-between gap-4 rounded-xl bg-[var(--surface-muted)] px-4 py-3"
                >
                  <div className="flex-1">
                    <div className="text-sm font-medium text-[var(--foreground)]">
                      {shortcut.action}
                    </div>
                    {statusInfo && (
                      <div className={`inline-flex items-center gap-1.5 mt-1 px-2 py-0.5 rounded text-xs font-medium ${statusInfo.tone === 'violet' ? 'bg-[var(--violet-soft)] text-[var(--violet-ink)]' : 
                        statusInfo.tone === 'blue' ? 'bg-[var(--blue-soft)] text-[var(--blue-ink)]' :
                        statusInfo.tone === 'amber' ? 'bg-[var(--amber-soft)] text-[var(--amber-ink)]' :
                        statusInfo.tone === 'green' ? 'bg-[var(--green-soft)] text-[var(--green-ink)]' :
                        'bg-[var(--stone-soft)] text-[var(--stone-ink)]'}`}>
                        <span className="w-1.5 h-1.5 rounded-full bg-current"></span>
                        {statusInfo.label}
                      </div>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5 flex-shrink-0">
                    {shortcut.keys.map((key, keyIndex) => (
                      <div key={keyIndex} className="flex items-center">
                        <kbd className="keyboard-shortcut">{key}</kbd>
                        {keyIndex < shortcut.keys.length - 1 && (
                          <span className="mx-1 text-xs text-[var(--faint)]">+</span>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              );
            })}
          </div>

          <div className="mt-6 pt-4 border-t border-[var(--border)]">
            <p className="text-xs text-[var(--muted)] text-center">
              Press <kbd className="keyboard-shortcut mx-1">Ctrl</kbd> + <kbd className="keyboard-shortcut mx-1">/</kbd> or <kbd className="keyboard-shortcut mx-1">Esc</kbd> to close
            </p>
          </div>
        </div>
      </div>
    </>
  );
}

// Hook for keyboard shortcuts in job list
export function useKeyboardShortcuts(onStatusChange?: (status: JobStatus) => void) {
  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      // Only trigger if we're not in an input field
      if (event.target instanceof HTMLInputElement || 
          event.target instanceof HTMLTextAreaElement ||
          event.target instanceof HTMLSelectElement) {
        return;
      }

      // Number keys for status changes (when onStatusChange is provided)
      if (onStatusChange && !event.ctrlKey && !event.metaKey && !event.altKey) {
        const statusMap: Record<string, JobStatus> = {
          '1': 'wishlist',
          '2': 'applied',
          '3': 'interviewing',
          '4': 'offer',
          '5': 'rejected',
        };

        const status = statusMap[event.key];
        if (status) {
          event.preventDefault();
          onStatusChange(status);
          
          // Visual feedback
          const feedback = document.createElement('div');
          feedback.className = `fixed top-4 right-4 z-50 px-4 py-2 rounded-lg font-medium animate-slide-in-right ${
            status === 'wishlist' ? 'bg-[var(--violet-soft)] text-[var(--violet-ink)]' :
            status === 'applied' ? 'bg-[var(--blue-soft)] text-[var(--blue-ink)]' :
            status === 'interviewing' ? 'bg-[var(--amber-soft)] text-[var(--amber-ink)]' :
            status === 'offer' ? 'bg-[var(--green-soft)] text-[var(--green-ink)]' :
            'bg-[var(--stone-soft)] text-[var(--stone-ink)]'
          }`;
          feedback.textContent = `Status changed to ${jobStatusPresentation(status).label}`;
          document.body.appendChild(feedback);
          
          setTimeout(() => {
            feedback.classList.add('opacity-0', 'transition-opacity', 'duration-300');
            setTimeout(() => feedback.remove(), 300);
          }, 2000);
        }
      }

      // Ctrl/Cmd + K for quick status popup
      if ((event.ctrlKey || event.metaKey) && event.key === 'k') {
        event.preventDefault();
        // This would trigger a global quick status change component
        // Could be implemented with a context or global state
        console.log('Quick status change triggered');
      }

      // F key for favorite toggle
      if (event.key === 'f' || event.key === 'F') {
        event.preventDefault();
        console.log('Toggle favorite triggered');
      }

      // A key for archive toggle
      if (event.key === 'a' || event.key === 'A') {
        event.preventDefault();
        console.log('Archive toggle triggered');
      }
    }

    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onStatusChange]);
}
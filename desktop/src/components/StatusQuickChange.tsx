import { useState, useRef, useEffect } from "react";
import { CheckIcon, ChevronDownIcon } from "@/components/icons";
import { jobStatuses, type JobStatus } from "@/lib/schema";
import { jobStatusPresentation, toneClasses } from "@/lib/ui";

interface StatusQuickChangeProps {
  currentStatus: JobStatus;
  onStatusChange: (newStatus: JobStatus) => void;
  size?: "sm" | "md" | "lg";
  showLabel?: boolean;
  className?: string;
}

export function StatusQuickChange({
  currentStatus,
  onStatusChange,
  size = "md",
  showLabel = true,
  className = "",
}: StatusQuickChangeProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [selectedStatus, setSelectedStatus] = useState<JobStatus>(currentStatus);
  const [showSuccess, setShowSuccess] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const currentInfo = jobStatusPresentation(currentStatus);
  const selectedInfo = jobStatusPresentation(selectedStatus);

  const sizeClasses = {
    sm: "px-2 py-1 text-xs",
    md: "px-3 py-1.5 text-sm",
    lg: "px-4 py-2 text-base",
  };

  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
        setIsOpen(false);
      }
    }

    if (isOpen) {
      document.addEventListener("mousedown", handleClickOutside);
    }
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [isOpen]);

  const handleStatusSelect = (status: JobStatus) => {
    if (status === currentStatus) {
      setIsOpen(false);
      return;
    }

    setSelectedStatus(status);
    
    // Show success animation
    setShowSuccess(true);
    setTimeout(() => setShowSuccess(false), 1500);
    
    // Call the change handler
    onStatusChange(status);
    
    // Close dropdown after a brief delay
    setTimeout(() => setIsOpen(false), 300);
  };

  // Status icons for visual representation
  const statusIcons: Record<JobStatus, string> = {
    wishlist: "⭐",
    applied: "📨",
    interviewing: "💬",
    offer: "🏆",
    rejected: "❌",
    withdrawn: "↩️",
    closed: "🔒",
    archived: "🗃️",
  };

  return (
    <div className={`relative inline-block ${className}`} ref={dropdownRef}>
      {/* Main trigger button */}
      <button
        type="button"
        onClick={() => setIsOpen(!isOpen)}
        className={`
          ${sizeClasses[size]}
          ${toneClasses[currentInfo.tone]}
          inline-flex items-center gap-2 rounded-lg font-medium transition-all
          hover:scale-[1.02] active:scale-[0.98] hover:shadow-md
          border border-transparent hover:border-current/30
          focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-[var(--accent)]
          ${showSuccess ? "animate-pulse" : ""}
        `}
        aria-expanded={isOpen}
        aria-label={`Change status from ${currentInfo.label}. Current status: ${currentInfo.label}`}
      >
        {showSuccess ? (
          <>
            <span className="animate-bounce">
              <CheckIcon size={size === "sm" ? 12 : size === "md" ? 14 : 16} />
            </span>
            <span className="animate-fade-in">Saved!</span>
          </>
        ) : (
          <>
            <span className="text-lg leading-none" aria-hidden="true">
              {statusIcons[currentStatus]}
            </span>
            {showLabel && <span>{currentInfo.label}</span>}
            <ChevronDownIcon
              size={size === "sm" ? 10 : size === "md" ? 12 : 14}
              className={`transition-transform duration-200 ${isOpen ? "rotate-180" : ""}`}
            />
          </>
        )}
      </button>

      {/* Dropdown menu with animation */}
      {isOpen && (
        <div
          className={`
            absolute z-50 mt-2 min-w-[200px] origin-top-left animate-scale-in
            rounded-xl border border-[var(--border)] bg-[var(--surface)]
            p-2 shadow-[var(--shadow-lg)] backdrop-blur-sm
            ${size === "sm" ? "left-0" : "left-0"}
          `}
          role="menu"
          aria-label="Status options"
        >
          <div className="mb-2 px-2 py-1">
            <p className="text-xs font-semibold text-[var(--muted)] uppercase tracking-wider">
              Change Status
            </p>
            <p className="text-xs text-[var(--faint)]">
              Click to update job stage
            </p>
          </div>
          
          <div className="space-y-1">
            {jobStatuses.map((status) => {
              const info = jobStatusPresentation(status);
              const isCurrent = status === currentStatus;
              const isSelected = status === selectedStatus;
              
              return (
                <button
                  key={status}
                  type="button"
                  onClick={() => handleStatusSelect(status)}
                  className={`
                    ${toneClasses[info.tone]}
                    w-full text-left px-3 py-2 rounded-lg text-sm font-medium
                    flex items-center gap-3 transition-all duration-150
                    hover:scale-[1.02] hover:shadow-sm
                    ${isCurrent ? "ring-2 ring-current ring-opacity-30" : ""}
                    ${isSelected ? "ring-2 ring-[var(--accent)]" : ""}
                    transform-gpu
                  `}
                  role="menuitem"
                  aria-current={isCurrent ? "true" : undefined}
                >
                  <span className="text-base" aria-hidden="true">
                    {statusIcons[status]}
                  </span>
                  <span className="flex-1">{info.label}</span>
                  {isCurrent && (
                    <span className="text-xs opacity-70">(current)</span>
                  )}
                </button>
              );
            })}
          </div>

          {/* Quick tips */}
          <div className="mt-3 border-t border-[var(--border)] pt-2 px-2">
            <p className="text-xs text-[var(--faint)]">
              <span className="font-medium text-[var(--accent)]">Tip:</span> Press Escape to close
            </p>
          </div>
        </div>
      )}

      {/* Success toast animation */}
      {showSuccess && (
        <div className="absolute -top-8 left-1/2 -translate-x-1/2 z-50">
          <div className="animate-slide-up animate-fade-in">
            <div className={`${toneClasses[selectedInfo.tone]} px-3 py-1.5 rounded-lg text-xs font-medium flex items-center gap-2 whitespace-nowrap shadow-lg`}>
              <CheckIcon size={12} className="animate-bounce" />
              <span>Changed to {selectedInfo.label}</span>
            </div>
            <div className="absolute -bottom-1 left-1/2 -translate-x-1/2 w-0 h-0 border-l-[6px] border-r-[6px] border-t-[6px] border-l-transparent border-r-transparent border-t-current"></div>
          </div>
        </div>
      )}
    </div>
  );
}

// Add these CSS animations to your index.css
export const statusChangeAnimations = `
@keyframes scale-in {
  from {
    opacity: 0;
    transform: scale(0.95);
  }
  to {
    opacity: 1;
    transform: scale(1);
  }
}

@keyframes slide-up {
  from {
    opacity: 0;
    transform: translateY(10px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes fade-in {
  from {
    opacity: 0;
  }
  to {
    opacity: 1;
  }
}

@keyframes bounce {
  0%, 100% {
    transform: translateY(0);
  }
  50% {
    transform: translateY(-4px);
  }
}

.animate-scale-in {
  animation: scale-in 0.15s ease-out;
}

.animate-slide-up {
  animation: slide-up 0.2s ease-out;
}

.animate-fade-in {
  animation: fade-in 0.2s ease-out;
}

.animate-bounce {
  animation: bounce 0.5s ease-in-out;
}

.animate-pulse {
  animation: pulse 0.5s ease-in-out;
}

@keyframes pulse {
  0%, 100% {
    opacity: 1;
  }
  50% {
    opacity: 0.7;
  }
}

.transform-gpu {
  transform: translateZ(0);
}
`;
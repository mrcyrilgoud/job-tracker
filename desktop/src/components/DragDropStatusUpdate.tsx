import { useState, useRef, useEffect } from "react";
import { DragIcon } from "@/components/icons";
import { jobStatuses, type JobStatus, type JobListSummary } from "@/lib/schema";
import { jobStatusPresentation, toneClasses } from "@/lib/ui";

interface DragDropStatusUpdateProps {
  jobs: JobListSummary[];
  onStatusChange: (jobId: string, newStatus: JobStatus) => void;
  children: (props: {
    isDragging: boolean;
    dragOverStatus: JobStatus | null;
    handleDragStart: (e: React.DragEvent, jobId: string, currentStatus: JobStatus) => void;
    handleDragOver: (e: React.DragEvent, status: JobStatus) => void;
    handleDrop: (e: React.DragEvent, targetStatus: JobStatus) => void;
    handleDragEnd: () => void;
  }) => React.ReactNode;
}

export function DragDropStatusUpdate({
  jobs,
  onStatusChange,
  children,
}: DragDropStatusUpdateProps) {
  const [draggingJobId, setDraggingJobId] = useState<string | null>(null);
  const [draggingJobStatus, setDraggingJobStatus] = useState<JobStatus | null>(null);
  const [dragOverStatus, setDragOverStatus] = useState<JobStatus | null>(null);
  const [isDragging, setIsDragging] = useState(false);
  const dragImageRef = useRef<HTMLDivElement | null>(null);

  // Create a custom drag image
  useEffect(() => {
    const dragImage = document.createElement('div');
    dragImage.style.position = 'absolute';
    dragImage.style.top = '-1000px';
    dragImage.style.left = '-1000px';
    dragImage.style.width = '200px';
    dragImage.style.padding = '12px';
    dragImage.style.borderRadius = '12px';
    dragImage.style.boxShadow = '0 8px 32px rgba(0, 0, 0, 0.2)';
    dragImage.style.backdropFilter = 'blur(12px)';
    dragImage.style.backgroundColor = 'var(--surface)';
    dragImage.style.border = '2px solid var(--accent)';
    dragImage.style.opacity = '0.9';
    dragImage.style.zIndex = '9999';
    dragImage.style.pointerEvents = 'none';
    dragImage.style.transform = 'rotate(3deg)';
    dragImageRef.current = dragImage;
    document.body.appendChild(dragImage);

    return () => {
      if (dragImageRef.current) {
        document.body.removeChild(dragImageRef.current);
      }
    };
  }, []);

  const handleDragStart = (e: React.DragEvent, jobId: string, currentStatus: JobStatus) => {
    const job = jobs.find(j => j.job.id === jobId);
    if (!job) return;

    setDraggingJobId(jobId);
    setDraggingJobStatus(currentStatus);
    setIsDragging(true);

    // Set custom drag image
    if (dragImageRef.current) {
      const statusInfo = jobStatusPresentation(currentStatus);
      dragImageRef.current.innerHTML = `
        <div style="display: flex; align-items: center; gap: 8px;">
          <div style="width: 8px; height: 8px; border-radius: 50%; background: var(--${statusInfo.tone}-ink);"></div>
          <div style="font-weight: 600; color: var(--foreground);">${job.job.title}</div>
        </div>
        <div style="font-size: 12px; color: var(--muted); margin-top: 4px;">${job.companyName}</div>
        <div style="font-size: 11px; color: var(--accent); margin-top: 8px; font-weight: 500;">Drag to change status</div>
      `;
      
      // Update styles based on status
      dragImageRef.current.style.borderColor = `var(--${statusInfo.tone}-ink)`;
      dragImageRef.current.style.background = `linear-gradient(135deg, var(--${statusInfo.tone}-soft), var(--surface))`;

      e.dataTransfer.setDragImage(dragImageRef.current, 100, 50);
    }

    // Set data for the drag operation
    e.dataTransfer.setData('application/job-tracker', JSON.stringify({
      jobId,
      currentStatus,
      title: job.job.title,
      companyName: job.companyName
    }));
    
    e.dataTransfer.effectAllowed = 'move';

    // Add visual feedback to the dragged element
    e.currentTarget.classList.add('dragging-source');
  };

  const handleDragOver = (e: React.DragEvent, status: JobStatus) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    
    if (draggingJobStatus !== status) {
      setDragOverStatus(status);
      
      // Add visual feedback to drop target
      e.currentTarget.classList.add('drag-over-active');
    }
  };

  const handleDragLeave = (e: React.DragEvent) => {
    e.currentTarget.classList.remove('drag-over-active');
    setDragOverStatus(null);
  };

  const handleDrop = (e: React.DragEvent, targetStatus: JobStatus) => {
    e.preventDefault();
    
    try {
      const data = JSON.parse(e.dataTransfer.getData('application/job-tracker'));
      const { jobId, currentStatus } = data;

      if (currentStatus !== targetStatus) {
        // Animate the drop
        e.currentTarget.classList.add('drop-success');
        setTimeout(() => {
          e.currentTarget.classList.remove('drop-success');
        }, 500);

        // Trigger the status change
        onStatusChange(jobId, targetStatus);
        
        // Show success animation
        showDropSuccessAnimation(targetStatus);
      }
    } catch (error) {
      console.error('Drop error:', error);
    }

    // Clean up
    e.currentTarget.classList.remove('drag-over-active');
    setDragOverStatus(null);
    setDraggingJobId(null);
    setDraggingJobStatus(null);
    setIsDragging(false);
  };

  const handleDragEnd = () => {
    // Clean up all drag states
    setDraggingJobId(null);
    setDraggingJobStatus(null);
    setDragOverStatus(null);
    setIsDragging(false);
    
    // Remove visual feedback from all elements
    document.querySelectorAll('.dragging-source').forEach(el => {
      el.classList.remove('dragging-source');
    });
    document.querySelectorAll('.drag-over-active').forEach(el => {
      el.classList.remove('drag-over-active');
    });
  };

  const showDropSuccessAnimation = (status: JobStatus) => {
    const statusInfo = jobStatusPresentation(status);
    const animation = document.createElement('div');
    
    animation.style.position = 'fixed';
    animation.style.top = '50%';
    animation.style.left = '50%';
    animation.style.transform = 'translate(-50%, -50%)';
    animation.style.zIndex = '9999';
    animation.style.padding = '20px 24px';
    animation.style.borderRadius = '16px';
    animation.style.background = `linear-gradient(135deg, var(--${statusInfo.tone}-soft), color-mix(in srgb, var(--${statusInfo.tone}-soft) 80%, white))`;
    animation.style.color = `var(--${statusInfo.tone}-ink)`;
    animation.style.fontWeight = '600';
    animation.style.fontSize = '16px';
    animation.style.boxShadow = '0 16px 48px rgba(0, 0, 0, 0.2)';
    animation.style.border = `2px solid color-mix(in srgb, var(--${statusInfo.tone}-ink) 30%, transparent)`;
    animation.style.opacity = '0';
    animation.style.transform = 'translate(-50%, -50%) scale(0.8)';
    animation.style.transition = 'all 0.3s cubic-bezier(0.34, 1.56, 0.64, 1)';
    animation.textContent = `✓ Status updated to ${statusInfo.label}`;
    
    document.body.appendChild(animation);

    // Trigger animation
    requestAnimationFrame(() => {
      animation.style.opacity = '1';
      animation.style.transform = 'translate(-50%, -50%) scale(1)';
    });

    // Remove after animation
    setTimeout(() => {
      animation.style.opacity = '0';
      animation.style.transform = 'translate(-50%, -50%) scale(1.1)';
      setTimeout(() => {
        if (animation.parentNode) {
          animation.parentNode.removeChild(animation);
        }
      }, 300);
    }, 1500);
  };

  return (
    <>
      {children({
        isDragging,
        dragOverStatus,
        handleDragStart,
        handleDragOver,
        handleDrop,
        handleDragEnd,
      })}
      
      {/* Global drag overlay */}
      {isDragging && (
        <div className="fixed inset-0 z-40 pointer-events-none">
          <div className="absolute inset-0 bg-gradient-to-br from-[var(--accent-soft)]/10 to-transparent"></div>
          <div className="absolute top-4 left-1/2 -translate-x-1/2 px-4 py-2 rounded-full bg-[var(--surface)] border border-[var(--accent)] shadow-lg animate-pulse">
            <div className="flex items-center gap-2 text-sm text-[var(--accent-ink)]">
              <DragIcon size={14} />
              <span>Drag job to another status column</span>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

// CSS to add to index.css
export const dragDropStyles = `
.dragging-source {
  opacity: 0.5;
  transform: scale(0.98);
  transition: all 0.2s ease;
}

.drag-over-active {
  position: relative;
  animation: drag-over-pulse 1s infinite;
  border-color: var(--accent) !important;
}

.drag-over-active::before {
  content: '';
  position: absolute;
  inset: -4px;
  border: 2px dashed var(--accent);
  border-radius: inherit;
  animation: drag-over-border 1.5s infinite;
}

.drop-success {
  animation: drop-bounce 0.5s cubic-bezier(0.34, 1.56, 0.64, 1);
}

@keyframes drag-over-pulse {
  0%, 100% {
    background-color: transparent;
  }
  50% {
    background-color: color-mix(in srgb, var(--accent-soft) 30%, transparent);
  }
}

@keyframes drag-over-border {
  0% {
    opacity: 0.3;
    transform: scale(1);
  }
  50% {
    opacity: 0.7;
    transform: scale(1.02);
  }
  100% {
    opacity: 0.3;
    transform: scale(1);
  }
}

@keyframes drop-bounce {
  0% {
    transform: scale(1);
  }
  30% {
    transform: scale(1.05);
  }
  50% {
    transform: scale(0.98);
  }
  70% {
    transform: scale(1.02);
  }
  100% {
    transform: scale(1);
  }
}

.drag-handle {
  cursor: grab;
  opacity: 0.4;
  transition: opacity 0.2s ease;
}

.drag-handle:hover {
  opacity: 0.8;
}

.drag-handle:active {
  cursor: grabbing;
}

.status-column {
  transition: all 0.3s ease;
}

.status-column.drag-target {
  transform: translateY(-4px);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.15);
}
`;
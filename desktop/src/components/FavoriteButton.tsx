import React from "react";
import { StarIcon } from "@/components/icons";

export function FavoriteButton({
  isFavorite,
  onToggle,
  size = 18,
  className = "",
  disabled = false,
}: {
  isFavorite: boolean;
  onToggle: (e: React.MouseEvent) => void;
  size?: number;
  className?: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onToggle(e);
      }}
      aria-label={isFavorite ? "Remove from favorites" : "Add to favorites"}
      title={isFavorite ? "Starred favorite" : "Star this job"}
      className={`inline-flex items-center justify-center rounded-lg p-1.5 transition-all duration-150 active:scale-90 hover:scale-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] ${
        isFavorite
          ? "text-amber-500 hover:bg-[var(--amber-soft)]"
          : "text-[var(--faint)] hover:bg-[var(--surface-muted)] hover:text-amber-500"
      } ${className}`}
    >
      <StarIcon size={size} filled={isFavorite} />
    </button>
  );
}

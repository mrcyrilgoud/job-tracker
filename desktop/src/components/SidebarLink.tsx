import { Link } from "react-router-dom";

export type SidebarLinkProps = {
  to: string;
  label: string;
  count?: number;
  /** Text shown in the count slot, with a screen-reader label. Takes precedence over `count`. */
  annotation?: { text: string; label: string };
  active?: boolean;
  Icon?: (props: { size?: number; className?: string; filled?: boolean }) => React.JSX.Element;
};

/** Sidebar navigation row with an optional icon and right-aligned count or annotation. */
export function SidebarLink({ to, label, count, annotation, active, Icon }: SidebarLinkProps) {
  const trailingClass = `font-display text-sm ${active ? "text-[var(--accent-ink)]" : "text-[var(--faint)]"}`;
  return (
    <Link
      to={to}
      className={`flex items-center justify-between rounded-lg px-3 py-2 text-sm transition-colors ${
        active
          ? "bg-[var(--accent-soft)] font-medium text-[var(--accent-ink)]"
          : "text-[var(--foreground)] hover:bg-[var(--surface-muted)]"
      }`}
    >
      <span className="flex items-center gap-2">
        {Icon ? <Icon size={14} className={active ? "text-[var(--accent-ink)]" : "text-[var(--faint)]"} filled={active} /> : null}
        <span>{label}</span>
      </span>
      {annotation ? (
        <span className={trailingClass}>
          <span aria-hidden="true">{annotation.text}</span>
          <span className="sr-only">{annotation.label}</span>
        </span>
      ) : typeof count === "number" ? (
        <span className={trailingClass}>{count}</span>
      ) : null}
    </Link>
  );
}

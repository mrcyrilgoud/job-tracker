import type { JobStatus } from "@/lib/schema";

type IconProps = {
  size?: number;
  className?: string;
};

function svgProps(size: number) {
  return {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.6,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    "aria-hidden": true,
  };
}

export function BriefcaseIcon({ size = 16, className }: IconProps) {
  return (
    <svg {...svgProps(size)} className={className}>
      <rect x="3" y="7" width="18" height="13" rx="2" />
      <path d="M8 7V5a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
      <path d="M3 12h18" />
    </svg>
  );
}

export function DocumentIcon({ size = 16, className }: IconProps) {
  return (
    <svg {...svgProps(size)} className={className}>
      <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" />
      <path d="M14 3v5h5" />
    </svg>
  );
}

export function BuildingIcon({ size = 16, className }: IconProps) {
  return (
    <svg {...svgProps(size)} className={className}>
      <rect x="4" y="3" width="16" height="18" rx="1.5" />
      <path d="M9 8h.01M15 8h.01M9 12h.01M15 12h.01M10 21v-3.5a2 2 0 0 1 4 0V21" />
    </svg>
  );
}

export function MailIcon({ size = 16, className }: IconProps) {
  return (
    <svg {...svgProps(size)} className={className}>
      <rect x="3" y="5" width="18" height="14" rx="2" />
      <path d="m4 7 8 5 8-5" />
    </svg>
  );
}

export function BookmarkIcon({ size = 16, className }: IconProps) {
  return (
    <svg {...svgProps(size)} className={className}>
      <path d="M6 4h12v16l-6-4-6 4z" />
    </svg>
  );
}

export function SendIcon({ size = 16, className }: IconProps) {
  return (
    <svg {...svgProps(size)} className={className}>
      <path d="M22 2 11 13" />
      <path d="M22 2 15 22l-4-9-9-4z" />
    </svg>
  );
}

export function ChatIcon({ size = 16, className }: IconProps) {
  return (
    <svg {...svgProps(size)} className={className}>
      <path d="M21 15a2 2 0 0 1-2 2H8l-4 4V5a2 2 0 0 1 2-2h13a2 2 0 0 1 2 2z" />
    </svg>
  );
}

export function TrophyIcon({ size = 16, className }: IconProps) {
  return (
    <svg {...svgProps(size)} className={className}>
      <path d="M6 4h12v4a6 6 0 0 1-12 0z" />
      <path d="M6 6H4a2 2 0 0 0 0 4h2M18 6h2a2 2 0 0 1 0 4h-2M9 20h6M12 14v6" />
    </svg>
  );
}

export function ArchiveIcon({ size = 16, className }: IconProps) {
  return (
    <svg {...svgProps(size)} className={className}>
      <rect x="3" y="4" width="18" height="4" rx="1" />
      <path d="M5 8v11a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V8M10 12h4" />
    </svg>
  );
}

export function BellIcon({ size = 16, className }: IconProps) {
  return (
    <svg {...svgProps(size)} className={className}>
      <path d="M18 8a6 6 0 1 0-12 0c0 7-3 9-3 9h18s-3-2-3-9" />
      <path d="M13.7 21a2 2 0 0 1-3.4 0" />
    </svg>
  );
}

type IconComponent = (props: IconProps) => React.JSX.Element;

export const statusIcons: Record<JobStatus, IconComponent> = {
  wishlist: BookmarkIcon,
  applied: SendIcon,
  interviewing: ChatIcon,
  offer: TrophyIcon,
  rejected: ArchiveIcon,
  withdrawn: ArchiveIcon,
  closed: ArchiveIcon,
};

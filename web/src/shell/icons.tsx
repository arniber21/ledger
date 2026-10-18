import type { ReactNode } from "react";

type IconProps = { className?: string };

export function Mark() {
  return (
    <svg className="mark" width="26" height="26" viewBox="0 0 26 26" aria-hidden="true">
      <rect width="26" height="26" rx="7" fill="#22262B" />
      <path
        d="M8 17V11.5M13 17V8.5M18 17V13"
        stroke="white"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  );
}

function Stroke({ className, children }: IconProps & { children: ReactNode }) {
  return (
    <svg className={className} width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
      {children}
    </svg>
  );
}

export function SearchIcon() {
  return (
    <Stroke>
      <circle cx="7" cy="7" r="4.25" stroke="currentColor" strokeWidth="1.4" />
      <path d="M10.5 10.5L13.5 13.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </Stroke>
  );
}

export function InboxIcon() {
  return (
    <Stroke>
      <path
        d="M2.5 9.5 4.2 3.8A1 1 0 0 1 5.15 3h5.7a1 1 0 0 1 .95.8L13.5 9.5"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
      <path
        d="M2.5 9.5h2.4a1.4 1.4 0 0 0 1.3.9h3.6a1.4 1.4 0 0 0 1.3-.9h2.4V12a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1V9.5Z"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
    </Stroke>
  );
}

export function DashboardIcon() {
  return (
    <Stroke>
      <rect x="2.5" y="2.5" width="4.5" height="4.5" rx="1" stroke="currentColor" strokeWidth="1.4" />
      <rect x="9" y="2.5" width="4.5" height="4.5" rx="1" stroke="currentColor" strokeWidth="1.4" />
      <rect x="2.5" y="9" width="4.5" height="4.5" rx="1" stroke="currentColor" strokeWidth="1.4" />
      <rect x="9" y="9" width="4.5" height="4.5" rx="1" stroke="currentColor" strokeWidth="1.4" />
    </Stroke>
  );
}

export function TracesIcon() {
  return (
    <Stroke>
      <path
        d="M6 3.5v3.2a1.5 1.5 0 0 0 .7 1.3L9 9.2a1.5 1.5 0 0 1 .7 1.3v2"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
      <circle cx="6" cy="3.2" r="1.2" stroke="currentColor" strokeWidth="1.3" />
      <circle cx="9.7" cy="12.8" r="1.2" stroke="currentColor" strokeWidth="1.3" />
      <path d="M10.2 4.2 13 3.2M10.5 6.8 13.2 7.6" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
    </Stroke>
  );
}

export function SpecsIcon() {
  return (
    <Stroke>
      <path
        d="M4.5 2.8h5.2L12.8 6v7.2a1 1 0 0 1-1 1H4.5a1 1 0 0 1-1-1V3.8a1 1 0 0 1 1-1Z"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
      <path d="M9.2 2.8V6H12.8M5.4 8.2h4.2M5.4 10.6h3" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
    </Stroke>
  );
}

export function VerdictsIcon() {
  return (
    <Stroke>
      <circle cx="8" cy="8" r="5.2" stroke="currentColor" strokeWidth="1.4" />
      <path d="M5.6 8.1 7.2 9.7 10.5 6.4" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
    </Stroke>
  );
}

export function ChevronIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
      <path d="M3 4.5 6 7.5 9 4.5" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function BoltIcon() {
  return (
    <Stroke>
      <path d="M8.6 1.8 3.4 8.2h3.2L6.2 14.2 12.2 6.6H8.8L8.6 1.8Z" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" />
    </Stroke>
  );
}

export function CheckIcon() {
  return (
    <Stroke>
      <path d="M3.2 8.2 6.3 11.2 12.8 4.2" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
    </Stroke>
  );
}

export function FilterIcon() {
  return (
    <Stroke>
      <path d="M2.5 3.5h11l-4 5.1v3.2l-3 1.2V8.6L2.5 3.5Z" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" />
    </Stroke>
  );
}

export function BoardIcon() {
  return (
    <Stroke>
      <rect x="2.2" y="2.4" width="3.2" height="11" rx="0.8" stroke="currentColor" strokeWidth="1.3" />
      <rect x="6.4" y="2.4" width="3.2" height="7.2" rx="0.8" stroke="currentColor" strokeWidth="1.3" />
      <rect x="10.6" y="2.4" width="3.2" height="9" rx="0.8" stroke="currentColor" strokeWidth="1.3" />
    </Stroke>
  );
}

export function SemanticIcon() {
  return (
    <Stroke>
      <path d="M3 4.2h6.2M3 8h10M3 11.8h7.4" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      <circle cx="12.4" cy="4.2" r="1.1" stroke="currentColor" strokeWidth="1.3" />
    </Stroke>
  );
}

export function ListIcon() {
  return (
    <Stroke>
      <path d="M3 4.2h10M3 8h10M3 11.8h10" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </Stroke>
  );
}

export function SplitIcon() {
  return (
    <Stroke>
      <rect x="2.2" y="2.6" width="11.6" height="10.8" rx="1.2" stroke="currentColor" strokeWidth="1.3" />
      <path d="M8 2.6v10.8" stroke="currentColor" strokeWidth="1.3" />
    </Stroke>
  );
}

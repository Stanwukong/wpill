/**
 * Cay's calendar mark: a plain line calendar in soft white (icons.tsx style), in the pill and the "starting soon"
 * activity. The author chose it over Google's product icon (D43), so no Google artwork ships with Cay.
 */
export function CalendarMark({ size = 16, className }: { size?: number; className?: string }) {
  return (
    <svg
      className={className ? `calendar-mark ${className}` : "calendar-mark"}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      role="img"
      aria-label="Calendar"
    >
      <rect x="3.5" y="5" width="17" height="15.5" rx="3" />
      <path d="M3.5 10h17M8 3v3.5M16 3v3.5" />
    </svg>
  );
}

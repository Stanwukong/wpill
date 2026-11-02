/**
 * Google's Calendar product icon, shown unaltered (full colour, only resized; Google's API brand rules: no recolouring,
 * filters, theme tint or merging with Cay's marks). The author drops Google's own file at
 * `src/assets/google-calendar.svg` (or `.png`); until it exists the glob is empty and Cay's own neutral calendar
 * glyph stands in. This is the one place that picks between them.
 */
const official = Object.values(
  import.meta.glob<string>("../../assets/google-calendar.{svg,png}", { eager: true, query: "?url", import: "default" }),
)[0];

export function CalendarMark({ size = 16, className }: { size?: number; className?: string }) {
  const cls = className ? `calendar-mark ${className}` : "calendar-mark";
  if (official) {
    return (
      <img className={cls} src={official} width={size} height={size} alt="Google Calendar" draggable={false} />
    );
  }
  // Cay's stand-in: a plain line calendar in soft white (icons.tsx style), deliberately unlike Google's design.
  return (
    <svg
      className={`${cls} calendar-mark--fallback`}
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

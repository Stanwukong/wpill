// Small line icons for activities. They inherit `currentColor`, so they follow the island theme.
import type { ReactNode } from "react";

function Icon({ children }: { children: ReactNode }) {
  return (
    <svg
      className="activity__icon"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

export function SpeakerIcon({ level, muted }: { level: number; muted: boolean }) {
  return (
    <Icon>
      <path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z" />
      {muted ? (
        <path d="M16 9.5l5 5M21 9.5l-5 5" />
      ) : (
        <>
          {level > 0 && <path d="M15.5 9.5a3.5 3.5 0 0 1 0 5" />}
          {level > 50 && <path d="M18 7a7 7 0 0 1 0 10" />}
        </>
      )}
    </Icon>
  );
}

export function SunIcon() {
  return (
    <Icon>
      <circle cx="12" cy="12" r="3.5" />
      <path d="M12 3v1.5M12 19.5V21M3 12h1.5M19.5 12H21M5.6 5.6l1.1 1.1M17.3 17.3l1.1 1.1M5.6 18.4l1.1-1.1M17.3 6.7l1.1-1.1" />
    </Icon>
  );
}

export function CapsLockIcon() {
  return (
    <Icon>
      <path d="M12 4.5l7 7h-3.5V15h-7v-3.5H5z" />
      <path d="M8.5 19h7" />
    </Icon>
  );
}

export function HeadphonesIcon() {
  return (
    <Icon>
      <path d="M4 15v-3a8 8 0 0 1 16 0v3" />
      <rect x="3.5" y="14" width="4" height="6" rx="1.5" />
      <rect x="16.5" y="14" width="4" height="6" rx="1.5" />
    </Icon>
  );
}

/** Battery outline with a fill proportional to `level`; a bolt while charging. */
export function BatteryIcon({ level, charging }: { level: number; charging: boolean }) {
  const fill = Math.max(1, Math.round((Math.min(100, level) / 100) * 13));
  return (
    <Icon>
      <rect x="2.5" y="7" width="17" height="10" rx="2.5" />
      <path d="M21.5 10.5v3" />
      {charging ? (
        <path d="M11.5 8.5l-2.5 4h3l-1.5 3" />
      ) : (
        <rect x="4.5" y="9" width={fill} height="6" rx="1" fill="currentColor" stroke="none" />
      )}
    </Icon>
  );
}

/** TopBar acting on the desktop for an agent run: a pointer. */
export function PointerIcon() {
  return (
    <Icon>
      <path d="M6 2.75v15.5l4.3-4 2.75 6 2.8-1.3-2.75-5.9h5.7z" />
    </Icon>
  );
}

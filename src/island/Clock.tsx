import { useEffect, useState } from "react";

const timeFormat = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const dateFormat = new Intl.DateTimeFormat(undefined, { weekday: "long", month: "long", day: "numeric" });

/** Current time, re-rendered at the top of each minute. */
function useMinuteClock(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    let timer: number;
    const schedule = () => {
      const msToNextMinute = 60_000 - (Date.now() % 60_000);
      timer = window.setTimeout(() => {
        setNow(new Date());
        schedule();
      }, msToNextMinute + 20);
    };
    schedule();
    return () => window.clearTimeout(timer);
  }, []);
  return now;
}

export function Clock({ variant }: { variant: "pill" | "panel" }) {
  const now = useMinuteClock();
  if (variant === "pill") {
    return <span className="clock clock--pill">{timeFormat.format(now)}</span>;
  }
  return (
    <div className="clock clock--panel">
      <span className="clock__time">{timeFormat.format(now)}</span>
      <span className="clock__date">{dateFormat.format(now)}</span>
    </div>
  );
}

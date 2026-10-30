import type { MediaSession } from "../../lib/types";
import { Artwork } from "./MediaCard";
import "./media.css";

/** Small art shown left of the clock in the collapsed pill while something plays. */
export function PillArt({ session }: { session: MediaSession }) {
  return <Artwork session={session} className="pill-art" />;
}

/** Pure CSS animation: WebView2 throttles JS timers while the island sits idle. */
export function Equalizer() {
  return (
    <span className="equalizer" aria-label="Playing">
      <span />
      <span />
      <span />
      <span />
    </span>
  );
}

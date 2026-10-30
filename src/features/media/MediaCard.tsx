import { useRef, useState, type MouseEvent } from "react";
import { commands, errorMessage } from "../../lib/ipc";
import type { MediaSession } from "../../lib/types";
import type { MediaController } from "./useMedia";
import { formatTime, useProgress } from "./useProgress";
import "./media.css";

export function MediaCard({ media }: { media: MediaController }) {
  const { sessions, active, select } = media;
  const [error, setError] = useState<string | null>(null);
  if (!active) return null;

  const run = (action: Promise<void>) => {
    setError(null);
    action.catch((err) => setError(errorMessage(err)));
  };

  return (
    <section className="media" aria-label="Now playing">
      <div className="media__main">
        <Artwork session={active} className="media__art" />
        <div className="media__text">
          <span className="media__title" title={active.title}>
            {active.title || active.appName}
          </span>
          <span className="media__artist">
            {[active.artist, active.appName].filter(Boolean).join(" · ")}
          </span>
        </div>
        <Controls session={active} run={run} />
      </div>
      {active.durationMs !== null && (
        // Keyed so a new session/track starts the bar from its own position.
        <Progress key={`${active.id}:${active.title}`} session={active} run={run} />
      )}
      {error && <p className="media__error">{error}</p>}
      {sessions.length > 1 && (
        <div className="media__sessions" role="tablist" aria-label="Media sessions">
          {sessions.map((s) => (
            <button
              key={s.id}
              role="tab"
              aria-selected={s.id === active.id}
              className="media__chip"
              title={s.title}
              onClick={() => select(s.id)}
            >
              {s.status === "playing" && <span className="media__chip-dot" aria-hidden />}
              {s.appName}
            </button>
          ))}
        </div>
      )}
    </section>
  );
}

export function Artwork({ session, className }: { session: MediaSession; className: string }) {
  return session.artwork ? (
    <img className={className} src={session.artwork} alt="" draggable={false} />
  ) : (
    <span className={`${className} media__art--fallback`} aria-hidden>
      <svg viewBox="0 0 24 24" width="45%" height="45%">
        <path fill="currentColor" d="M9 17.5a2.5 2.5 0 1 1-2-2.45V5l11-2v11.5a2.5 2.5 0 1 1-2-2.45V6.4l-7 1.3z" />
      </svg>
    </span>
  );
}

function Controls({ session, run }: { session: MediaSession; run: (p: Promise<void>) => void }) {
  const playing = session.status === "playing";
  return (
    <div className="media__controls">
      <button
        className="media__button"
        aria-label="Previous"
        disabled={!session.canPrev}
        onClick={() => run(commands.mediaPrev(session.id))}
      >
        <svg viewBox="0 0 24 24" aria-hidden>
          <path fill="currentColor" d="M6 5h2v14H6zm3.5 7L19 5v14z" />
        </svg>
      </button>
      <button
        className="media__button media__button--main"
        aria-label={playing ? "Pause" : "Play"}
        disabled={playing ? !session.canPause : !session.canPlay}
        onClick={() => run(commands.mediaPlayPause(session.id))}
      >
        <svg viewBox="0 0 24 24" aria-hidden>
          {playing ? (
            <path fill="currentColor" d="M6 5h4v14H6zm8 0h4v14h-4z" />
          ) : (
            <path fill="currentColor" d="M7 4.5v15L19.5 12z" />
          )}
        </svg>
      </button>
      <button
        className="media__button"
        aria-label="Next"
        disabled={!session.canNext}
        onClick={() => run(commands.mediaNext(session.id))}
      >
        <svg viewBox="0 0 24 24" aria-hidden>
          <path fill="currentColor" d="M16 5h2v14h-2zM5 5l9.5 7L5 19z" />
        </svg>
      </button>
    </div>
  );
}

function Progress({ session, run }: { session: MediaSession; run: (p: Promise<void>) => void }) {
  const barRef = useRef<HTMLDivElement>(null);
  const position = useProgress(session, barRef);
  const duration = session.durationMs ?? 0;

  const seek = (e: MouseEvent<HTMLDivElement>) => {
    if (!session.canSeek || duration <= 0) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const fraction = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
    run(commands.mediaSeek(session.id, fraction * duration));
  };

  return (
    <div className="media__progress">
      <span className="media__time">{formatTime(position)}</span>
      <div
        className="media__track"
        data-seekable={session.canSeek}
        role="slider"
        aria-label="Playback position"
        aria-valuemin={0}
        aria-valuemax={duration}
        aria-valuenow={position}
        onClick={seek}
      >
        <div ref={barRef} className="media__fill" />
      </div>
      <span className="media__time">{formatTime(duration)}</span>
    </div>
  );
}

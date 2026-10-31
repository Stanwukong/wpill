import type { ReactNode } from "react";
import type { ActivityPayload, PowerActivity } from "../../lib/types";
import { endLabel } from "../agent/labels";
import { openRun } from "../agent/useAgentRuns";
import { focusClaudeSession } from "../claude/useClaudeSessions";
import { AgentActing } from "./AgentActing";
import { AgentApproval } from "./AgentApproval";
import { ClaudeActivity } from "./ClaudeActivity";
import { BatteryIcon, CapsLockIcon, HeadphonesIcon, SpeakerIcon, SunIcon } from "./icons";
import { ScreenshotActivity } from "./ScreenshotActivity";
import "./activities.css";

/** The compact island content for one activity. Timing and hit testing live in Rust. */
export function ActivityView({ activity }: { activity: ActivityPayload }) {
  switch (activity.kind) {
    case "volume": {
      const { level, muted, device } = activity.payload;
      return (
        <Meter
          icon={<SpeakerIcon level={level} muted={muted} />}
          caption={device || "Volume"}
          level={muted ? 0 : level}
          value={muted ? "Muted" : String(level)}
        />
      );
    }
    case "brightness": {
      const { level } = activity.payload;
      return <Meter icon={<SunIcon />} caption="Brightness" level={level} value={String(level)} />;
    }
    case "capsLock": {
      const { on } = activity.payload;
      return (
        <Row icon={<CapsLockIcon />} title="Caps Lock">
          <span className="activity__state" data-on={on}>
            {on ? "On" : "Off"}
          </span>
        </Row>
      );
    }
    case "bluetooth": {
      const { name, connected, battery } = activity.payload;
      const detail = !connected ? "Disconnected" : battery != null ? `Connected · ${battery}%` : "Connected";
      return (
        <Row icon={<HeadphonesIcon />} title={name} dim={!connected}>
          <span className="activity__detail">{detail}</span>
        </Row>
      );
    }
    case "power":
      return <Power power={activity.payload} />;
    case "screenshot":
      return <ScreenshotActivity id={activity.id} shot={activity.payload} />;
    case "claudeDone": {
      const { sessionId, project, failed } = activity.payload;
      return (
        <ClaudeActivity
          project={project}
          state={failed ? "stopped" : "done"}
          onOpen={() => focusClaudeSession(sessionId)}
        />
      );
    }
    case "claudeNeedsInput": {
      const { sessionId, project } = activity.payload;
      return <ClaudeActivity project={project} state="needsInput" onOpen={() => focusClaudeSession(sessionId)} />;
    }
    case "agentApproval":
      return <AgentApproval approval={activity.payload} />;
    case "agentActing":
      return <AgentActing acting={activity.payload} />;
    case "agentFinished": {
      const { runId, project, status, reason } = activity.payload;
      // Only a completed run gets the check; limits, failures and stops read as "stopped" (no mark).
      return (
        <ClaudeActivity
          project={project}
          state={reason === "completed" || (reason === null && status === "done") ? "done" : "stopped"}
          label={endLabel(status, reason)}
          onOpen={() => openRun(runId)}
        />
      );
    }
  }
}

const POWER_TITLES: Record<PowerActivity["event"], string> = {
  pluggedIn: "Charging",
  unplugged: "On battery",
  low: "Low battery",
};

function Power({ power }: { power: PowerActivity }) {
  return (
    <Row
      icon={<BatteryIcon level={power.level} charging={power.event === "pluggedIn"} />}
      title={POWER_TITLES[power.event]}
      tone={power.event === "low" ? "alert" : power.event === "pluggedIn" ? "charge" : undefined}
    >
      <span className="activity__value">{power.level}%</span>
    </Row>
  );
}

function Row({
  icon,
  title,
  children,
  dim = false,
  tone,
}: {
  icon: ReactNode;
  title: string;
  children: ReactNode;
  dim?: boolean;
  /** Semantic color for the icon (and text): low battery, or charging. */
  tone?: "alert" | "charge";
}) {
  return (
    <div className="activity" data-dim={dim} data-tone={tone}>
      {icon}
      <span className="activity__title">{title}</span>
      {children}
    </div>
  );
}

function Meter({ icon, caption, level, value }: { icon: ReactNode; caption: string; level: number; value: string }) {
  return (
    <div className="activity">
      {icon}
      <div className="activity__meter">
        <span className="activity__caption">{caption}</span>
        <div className="activity__track">
          <div className="activity__fill" style={{ width: `${Math.max(0, Math.min(100, level))}%` }} />
        </div>
      </div>
      <span className="activity__value">{value}</span>
    </div>
  );
}

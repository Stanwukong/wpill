//! IPC contract shared with `src/lib/ipc.ts` and `src/lib/types.ts`. Change both sides together.

use serde::{Deserialize, Serialize};

pub mod events {
    pub const HOVER_ENTER: &str = "island:hover-enter";
    pub const HOVER_LEAVE: &str = "island:hover-leave";
    pub const VISIBILITY: &str = "island:visibility";
    pub const MEDIA_STATE: &str = "media:state";
    pub const ACTIVITY_SHOW: &str = "activity:show";
    pub const ACTIVITY_HIDE: &str = "activity:hide";
    pub const CLAUDE_SESSIONS: &str = "claude:sessions";
    pub const AGENT_RUNS: &str = "agent:runs";
    /// Sent to the `command` window only.
    pub const AGENT_OUTPUT: &str = "agent:output";
    /// Sent to the `command` window only: which run to select when it opens.
    pub const COMMAND_FOCUS_RUN: &str = "command:focus-run";
    pub const ROUTINES_CHANGED: &str = "routines:changed";
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityPayload {
    pub hidden: bool,
    pub reason: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandStatePayload {
    pub expanded: bool,
    pub hidden: bool,
    pub activity: Option<ActivityPayload>,
}

/// `activity:show`. Re-sent with the same `id` when a burst (e.g. volume key presses) updates it in place.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPayload {
    pub id: u32,
    #[serde(flatten)]
    pub activity: Activity,
    /// Time left at emit; Rust times it (and holds it while an interactive activity is hovered).
    /// `None` for a sticky notice, which stays until its source withdraws it.
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityHidePayload {
    pub id: u32,
}

/// Serialized as `{ kind, payload }`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "camelCase")]
pub enum Activity {
    Volume(VolumeActivity),
    Brightness(BrightnessActivity),
    Bluetooth(BluetoothActivity),
    Power(PowerActivity),
    CapsLock(CapsLockActivity),
    Screenshot(ScreenshotActivity),
    ClaudeDone(ClaudeDoneActivity),
    ClaudeNeedsInput(ClaudeNeedsInputActivity),
    AgentApproval(AgentApprovalActivity),
    AgentActing(AgentActingActivity),
    AgentFinished(AgentFinishedActivity),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeActivity {
    /// 0–100.
    pub level: u8,
    pub muted: bool,
    pub device: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrightnessActivity {
    /// 0–100.
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BluetoothActivity {
    pub name: String,
    pub connected: bool,
    /// 0–100, when the device reports it.
    pub battery: Option<u8>,
    /// Device container id; one headset's A2DP and hands-free endpoints share it.
    #[serde(skip)]
    pub container: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PowerEvent {
    PluggedIn,
    Unplugged,
    Low,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerActivity {
    pub event: PowerEvent,
    /// Battery percentage, 0–100.
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapsLockActivity {
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotActivity {
    pub file_name: String,
    /// PNG data URL, about 256 px on its long side.
    pub thumbnail: String,
    pub width: u32,
    pub height: u32,
    /// Stays in Rust: drag/open commands look it up by activity id, so JS never names a path.
    #[serde(skip)]
    pub path: std::path::PathBuf,
}

/// A Claude Code turn finished (`Stop`), or ended on an API error (`StopFailure`, `failed`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeDoneActivity {
    /// Ours (`ClaudeSession::session_id`), not Claude Code's id.
    pub session_id: u32,
    /// The session's working folder name.
    pub project: String,
    pub failed: bool,
}

/// A Claude Code session is waiting on a permission prompt or a question. Sticky: it stays until the
/// session's next event.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeNeedsInputActivity {
    pub session_id: u32,
    pub project: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaudeStatus {
    /// Open, no task yet (or a new one after `/clear`).
    Idle,
    Working,
    Done,
    NeedsInput,
}

/// `claude:sessions`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeSessionsPayload {
    /// Most recently updated first.
    pub sessions: Vec<ClaudeSession>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeSession {
    /// Ours, stable for the session's lifetime; Claude Code's id stays in Rust.
    pub session_id: u32,
    pub project: String,
    pub status: ClaudeStatus,
    /// Unix epoch ms of the last status change.
    pub updated_at_ms: i64,
}

/// How much an agent step can affect: reading, changing files or running commands, or input into other apps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentRisk {
    Read,
    Change,
    Input,
}

/// An agent run asks to use a tool. Sticky: it stays until it's answered, times out, or the run ends.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentApprovalActivity {
    pub request_id: u32,
    pub run_id: String,
    pub project: String,
    /// Display name, e.g. `Bash` or `github · create_issue`.
    pub tool: String,
    /// One line built in Rust: the command, the file, the host.
    pub summary: String,
    pub risk: AgentRisk,
    /// What "allow for this run" would cover ("git", "edits"), or `None` where it isn't offered (`input`
    /// actions, compound shell commands).
    pub remember_as: Option<String>,
    /// One of TopBar's desktop tools: a look (2.7c, risk `read`, an eye) or an action (2.7d, risk `input`, a
    /// pointer). The screen edges glow while it runs.
    pub desktop: bool,
    /// The full text an action would type or set, for the command window (the island shows `summary`).
    pub detail: Option<String>,
    /// Set for a routine's run (2.7e): shown in place of `project`.
    pub routine_name: Option<String>,
}

/// TopBar is acting on the desktop for a run (2.7d). Sticky and not interactive; withdrawn when the action ends.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentActingActivity {
    pub run_id: String,
    pub project: String,
    /// "Clicking Save", "Typing into Notepad".
    pub step: String,
    pub phase: AgentActingPhase,
}

/// `waiting`: TopBar acts once you've let go of the mouse and keyboard. `acting`: it's acting now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentActingPhase {
    Waiting,
    Acting,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentFinishedActivity {
    pub run_id: String,
    pub project: String,
    /// Set for a routine's run (2.7e): shown in place of `project`.
    pub routine_name: Option<String>,
    pub status: AgentStatus,
    pub reason: Option<AgentEndReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentStatus {
    Starting,
    Working,
    /// Waiting for an approval.
    Waiting,
    Done,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentEndReason {
    Completed,
    MaxTurns,
    MaxBudget,
    Error,
    Stopped,
}

/// `agent:runs`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunsPayload {
    /// Most recently started first.
    pub runs: Vec<AgentRun>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRun {
    pub run_id: String,
    /// Folder name of `cwd`.
    pub project: String,
    pub cwd: String,
    /// The first prompt, shortened, for the command window's run list.
    pub prompt: String,
    pub status: AgentStatus,
    /// Set once the run has ended.
    pub reason: Option<AgentEndReason>,
    /// What it's doing now ("Running git status"), while working.
    pub step: Option<String>,
    pub turns: u32,
    /// Claude Code's own estimate for the session.
    pub cost_usd: Option<f64>,
    pub model: Option<String>,
    /// Approvals waiting for an answer, oldest first.
    pub pending: Vec<AgentApprovalActivity>,
    /// Why it failed, when it did.
    pub error: Option<String>,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
    /// The routine that started it (2.7e), if any.
    pub routine: Option<AgentRunRoutine>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunRoutine {
    pub id: String,
    pub name: String,
    pub trigger: RoutineTrigger,
}

/// How a routine's run started: Run now, or its schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoutineTrigger {
    Manual,
    Schedule,
}

/// When a routine runs by itself, while TopBar is running (2.7e). Times are local `HH:MM`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Schedule {
    /// Only by hand.
    Off,
    /// At `time` on each of `days`.
    Days { time: String, days: Vec<Weekday> },
    /// Every `hours` hours (1–24), counted from `from` on the day the schedule was saved.
    Every { hours: u32, from: String },
    /// Two minutes after TopBar starts.
    Start,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

/// A due time that passed while TopBar wasn't running or the PC slept: run once when back, or skip it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IfMissed {
    Once,
    Skip,
}

/// Why a due time didn't start a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    /// Missed (TopBar closed, asleep) and set to skip, or more than a day ago.
    Missed,
    Paused,
    /// Its previous run hadn't ended.
    StillRunning,
    /// Two scheduled runs were already running.
    TooMany,
    FolderMissing,
    StartFailed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineLastRun {
    pub run_id: String,
    pub trigger: RoutineTrigger,
    pub at_ms: i64,
    /// `None` while it runs.
    pub status: Option<AgentStatus>,
    pub reason: Option<AgentEndReason>,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineSkip {
    pub at_ms: i64,
    /// The due time that was skipped.
    pub due_ms: i64,
    pub why: SkipReason,
}

/// A saved instruction (2.7e), as stored in `routines.json` and sent to the command window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: String,
    pub name: String,
    pub prompt: String,
    /// Long-path normalised.
    pub cwd: String,
    pub model: Option<String>,
    pub max_turns: u32,
    /// Always set: routines have a budget.
    pub max_budget_usd: f64,
    pub claude_path: Option<String>,
    pub schedule: Schedule,
    /// The schedule is on (Run now works either way).
    pub enabled: bool,
    pub if_missed: IfMissed,
    /// Scheduled runs: an approval's timeout is paused while you're away (locked, or no input for 5 min).
    pub wait_for_me: bool,
    /// TopBar's desktop looks and actions; off passes them in `--disallowedTools`.
    pub desktop_tools: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// When the schedule was last set or turned on: due times before it don't count; anchors `every`.
    pub schedule_since_ms: i64,
    /// The latest due time handled (run or skipped).
    pub last_due_ms: i64,
    pub last_run: Option<RoutineLastRun>,
    pub last_skip: Option<RoutineSkip>,
}

/// One routine in `routines:changed`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineView {
    #[serde(flatten)]
    pub routine: Routine,
    /// When its schedule runs it next (`None`: off, disabled or paused).
    pub next_run_at_ms: Option<i64>,
    /// Its run going now, if any.
    pub running_run_id: Option<String>,
}

/// `routines:changed` and `routines_list`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutinesPayload {
    pub routines: Vec<RoutineView>,
    pub paused: bool,
}

/// One transcript entry: what the command window shows for a run.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEntry {
    pub at_ms: i64,
    #[serde(flatten)]
    pub event: AgentEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AgentEvent {
    /// A prompt or follow-up you sent.
    User {
        text: String,
    },
    /// Claude's text.
    Text {
        text: String,
    },
    Tool {
        id: String,
        name: String,
        label: String,
    },
    ToolResult {
        id: String,
        is_error: bool,
        text: String,
    },
    Approval {
        request_id: u32,
        tool: String,
        summary: String,
        risk: AgentRisk,
    },
    Decision {
        request_id: u32,
        allow: bool,
        /// `user`, `remembered`, `timeout` or `runEnded`.
        by: &'static str,
    },
    Result {
        status: AgentStatus,
        reason: AgentEndReason,
        turns: u32,
        cost_usd: Option<f64>,
    },
    Error {
        message: String,
    },
}

/// `agent:output`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOutputPayload {
    pub run_id: String,
    pub entry: AgentEntry,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStarted {
    pub run_id: String,
}

/// A `claude.exe` TopBar found.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeInstall {
    pub path: String,
    /// `2.1.312`, or `None` if `--version` didn't answer.
    pub version: Option<String>,
    /// e.g. "npm", "VS Code", "native", "PATH".
    pub source: &'static str,
}

/// `agent_options`: what the command window offers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOptions {
    pub installs: Vec<ClaudeInstall>,
    /// The install a run uses unless one is picked.
    pub default_install: Option<String>,
    /// Most recent first: folders of earlier runs, then Claude Code's own projects.
    pub recent_folders: Vec<String>,
    pub model: Option<String>,
    pub max_turns: u32,
    pub max_budget_usd: Option<f64>,
    pub hotkey: String,
    /// Set when the hotkey couldn't be registered (taken by another app).
    pub hotkey_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeHooksStatus {
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub hwnd: i64,
    pub title: String,
    pub exe_name: Option<String>,
    /// PNG data URL.
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaStatePayload {
    /// Most recently active first.
    pub sessions: Vec<MediaSession>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSession {
    /// Ours, stable for the session's lifetime; not a Windows id.
    pub id: u32,
    pub app_name: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Image data URL.
    pub artwork: Option<String>,
    /// `"playing" | "paused" | "stopped" | "changing"`
    pub status: &'static str,
    /// Position at `updated_at_ms`; React interpolates from there while playing.
    pub position_ms: i64,
    pub duration_ms: Option<i64>,
    /// Unix epoch ms.
    pub updated_at_ms: i64,
    pub playback_rate: f64,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_next: bool,
    pub can_prev: bool,
    pub can_seek: bool,
}

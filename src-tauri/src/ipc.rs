//! IPC contract shared with `src/lib/ipc.ts` and `src/lib/types.ts`. Change both sides together.

use serde::{Deserialize, Serialize};

pub mod events {
    pub const HOVER_ENTER: &str = "island:hover-enter";
    pub const HOVER_LEAVE: &str = "island:hover-leave";
    pub const VISIBILITY: &str = "island:visibility";
    /// The island's monitor, resolution or DPI scale changed; React measures again (Phase 4a-2).
    pub const MONITOR_CHANGED: &str = "monitor:changed";
    pub const MEDIA_STATE: &str = "media:state";
    pub const ACTIVITY_SHOW: &str = "activity:show";
    pub const ACTIVITY_HIDE: &str = "activity:hide";
    pub const SESSIONS: &str = "sessions:changed";
    pub const AGENT_RUNS: &str = "agent:runs";
    /// Sent to the `command` window only.
    pub const AGENT_OUTPUT: &str = "agent:output";
    /// Claude's text as it's written (2.10a), command window only; the next `agent:output` entry replaces it.
    pub const AGENT_TEXT_DRAFT: &str = "agent:textDraft";
    /// Sent to the `command` window only: which run to select when it opens.
    pub const COMMAND_FOCUS_RUN: &str = "command:focus-run";
    pub const ROUTINES_CHANGED: &str = "routines:changed";
    /// Terminal sessions' held requests changed (2.8b): the command window lists them.
    pub const SESSION_ASKS: &str = "sessions:asks";
    /// Sent to the `command` window only: which held session request to show when it opens (2.8b).
    pub const COMMAND_FOCUS_ASK: &str = "command:focus-ask";
    /// Google sign-in state changed (Phase 6a).
    pub const GOOGLE_STATE: &str = "google:state";
    /// Upcoming events and due tasks changed, or one changed phase (Phase 6a).
    pub const CALENDAR_UPCOMING: &str = "calendar:upcoming";
    /// The memory-heavy apps, every 2 s while the panel is open and `processes.enabled` is on (Phase 3).
    pub const PROCESSES_HEAVY: &str = "processes:heavy";
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityPayload {
    pub hidden: bool,
    pub reason: Option<&'static str>,
}

/// `monitor:changed`: the island's new DPI scale (1.0 at 96 DPI, without Text size).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorChangedPayload {
    pub scale: f64,
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
    SessionDone(SessionDoneActivity),
    SessionNeedsInput(SessionNeedsInputActivity),
    SessionApproval(SessionApprovalActivity),
    AgentApproval(AgentApprovalActivity),
    AgentActing(AgentActingActivity),
    AgentFinished(AgentFinishedActivity),
    /// An event starts soon (Phase 6b): shown once, at `google.leadMinutes` before it.
    CalendarSoon(CalendarSoonActivity),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarSoonActivity {
    /// The event's id for `calendar_open` / `calendar_join`.
    pub event_id: String,
    pub title: String,
    pub starts_at_ms: i64,
    /// Whole minutes until it starts when offered ("in 5 min"); 0 = starting now. Rust works it out so React needs
    /// no clock.
    pub minutes: u32,
    pub has_join: bool,
    /// The calendar's colour in Google (`#rrggbb`).
    pub color: Option<String>,
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

/// Which coding agent a terminal session belongs to (2.9a, D47). On the pipe it defaults to Claude, so forwarders and
/// hook entries from before 2.9 keep working.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentKind {
    #[default]
    Claude,
    Codex,
    Opencode,
}

impl AgentKind {
    pub fn is_claude(&self) -> bool {
        *self == AgentKind::Claude
    }

    /// The name people know it by, for dialogs and logs.
    pub fn display(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude Code",
            AgentKind::Codex => "Codex",
            AgentKind::Opencode => "OpenCode",
        }
    }
}

/// A session's turn finished (`Stop`), or ended on an API error (`StopFailure`, `failed`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDoneActivity {
    /// Ours (`AgentSession::session_id`), not the agent's id.
    pub session_id: u32,
    pub agent: AgentKind,
    /// The session's working folder name.
    pub project: String,
    pub failed: bool,
}

/// A session is waiting on a permission prompt or a question. Sticky: it stays until the session's next event.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNeedsInputActivity {
    pub session_id: u32,
    pub agent: AgentKind,
    pub project: String,
}

/// A terminal session asks to use a tool (2.8a), held until you answer here, answer in the terminal, or it's handed
/// back (`sessions::asks::HOLD`). Sticky. Allow / Deny only: a terminal session isn't Cay's to widen.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionApprovalActivity {
    pub request_id: u32,
    /// The session's row, for focusing its window; `None` if Cay hasn't seen the session yet.
    pub session_id: Option<u32>,
    pub agent: AgentKind,
    pub project: String,
    /// Display name, as for agent runs (`Bash`, `github · create_issue`).
    pub tool: String,
    /// One line built in Rust (`agent::describe`).
    pub summary: String,
    pub risk: AgentRisk,
    /// The card's diff or wrapped command (2.8b); the whole one comes from `session_ask_preview`.
    pub preview: Option<Box<ApprovalPreview>>,
    /// A question or a plan (2.8c), answered with `session_answer` / `session_plan` instead of Allow.
    pub interaction: Option<Box<Interaction>>,
}

/// `command_take_focus`: what a newly created command window selects (its page missed the focus event).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CommandFocus {
    Run { run_id: String },
    Ask { request_id: u32 },
}

/// `sessions:asks` and `session_asks`: the session requests the island holds, oldest first.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAsksPayload {
    pub asks: Vec<SessionApprovalActivity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    /// Open, no task yet (or a new one after `/clear`).
    Idle,
    Working,
    Done,
    NeedsInput,
}

/// `sessions:changed`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsPayload {
    /// Most recently updated first.
    pub sessions: Vec<AgentSession>,
    /// The newest usage limits from any session's status line (2.8f).
    pub limits: Option<UsageLimits>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    /// Ours, stable for the session's lifetime; the agent's own id stays in Rust.
    pub session_id: u32,
    pub agent: AgentKind,
    pub project: String,
    pub status: SessionStatus,
    /// Unix epoch ms of the last status change.
    pub updated_at_ms: i64,
    /// The tool it's using and on what ("Edit · hover/logic.rs"), from `PreToolUse` (2.8d); `None` between tasks.
    pub step: Option<String>,
    /// Its to-do list's progress, from the latest `TodoWrite` (2.8d).
    pub todos: Option<TodoCount>,
    /// Its permission mode as its hooks report it (`default`, `acceptEdits`, `plan`, `auto`, …; 2.8e). Shown
    /// read-only: Cay never changes a terminal session's mode.
    pub mode: Option<String>,
}

/// An agent run's permission mode (2.8e, D40): Claude Code's own names. Auto by default; `bypassPermissions` and
/// The agent a run uses (2.9b, D47/D48). Claude Code is native (stream-json); the others speak the Agent Client
/// Protocol. Missing in requests and routines means Claude Code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentBackend {
    #[default]
    Claude,
    Opencode,
    Codex,
}

impl AgentBackend {
    pub fn display(self) -> &'static str {
        match self {
            AgentBackend::Claude => "Claude Code",
            AgentBackend::Opencode => "OpenCode",
            AgentBackend::Codex => "Codex",
        }
    }
}

/// One agent in the command window's picker (2.9c).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBackendInfo {
    pub backend: AgentBackend,
    /// Installed where Cay can start it.
    pub available: bool,
    /// How to install it, when it isn't.
    pub install: Option<String>,
    /// The model a task uses when it doesn't name one (`agent.model` / `agent.backends.<agent>.model`).
    pub default_model: Option<String>,
    /// The models the agent lists; empty means free text (Claude Code) or not known yet.
    pub models: Vec<String>,
}

/// `agent_backends`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBackendsPayload {
    /// `agent.defaultBackend`.
    pub default_backend: AgentBackend,
    pub backends: Vec<AgentBackendInfo>,
}

/// `dontAsk` are never offered (they don't deserialize).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentMode {
    /// Claude Code's classifier approves routine actions and blocks risky ones.
    Auto,
    /// Ask: every tool Claude Code would ask about comes to the island.
    Default,
    AcceptEdits,
    Plan,
}

fn ask_mode() -> AgentMode {
    AgentMode::Default
}

/// Claude usage limits (2.8f, D41), from Claude Code's status line input: the newest reading of any session.
/// A window is missing when Claude Code didn't report it (not a Pro/Max/Ultra plan, or before the first response).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimits {
    pub five_hour: Option<LimitWindow>,
    pub seven_day: Option<LimitWindow>,
    /// When the status line read it (Unix ms).
    pub at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    /// 0–100 (Claude Code's `used_percentage`).
    pub used_percent: f64,
    pub resets_at_ms: i64,
    /// Set by Rust once `resets_at_ms` has passed without a newer reading.
    #[serde(default)]
    pub stale: bool,
}

/// `TodoWrite`'s progress (2.8d): items completed and in all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoCount {
    pub done: u32,
    pub total: u32,
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
    /// One of Cay's desktop tools: a look (2.7c, risk `read`, an eye) or an action (2.7d, risk `input`, a
    /// pointer). The screen edges glow while it runs.
    pub desktop: bool,
    /// The full text an action would type or set, for the command window (the island shows `summary`).
    pub detail: Option<String>,
    /// Set for a routine's run (2.7e): shown in place of `project`.
    pub routine_name: Option<String>,
    /// The card's diff or wrapped command (2.8b); the whole one comes from `agent_approval_preview`.
    pub preview: Option<Box<ApprovalPreview>>,
    /// A question or a plan (2.8c), answered with `agent_answer` / `agent_plan` instead of Allow.
    pub interaction: Option<Box<Interaction>>,
}

/// `AskUserQuestion` or `ExitPlanMode` (2.8c): what the card shows instead of Allow / Deny.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Interaction {
    /// The questions and the answers given so far (question text → label, or labels joined by ", ").
    Question {
        questions: Vec<QuestionView>,
        answers: std::collections::BTreeMap<String, String>,
    },
    /// The plan itself comes from the preview commands (`ApprovalPreview::Plan`).
    Plan { title: String, lines: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionView {
    /// A short label ("Framework").
    pub header: String,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

/// What an approval shows beyond its one line (2.8b), built by `agent::preview`. The card's copy is capped at a few
/// lines; the command window asks for the whole one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ApprovalPreview {
    Diff(DiffPreview),
    Command(CommandPreview),
    /// `ExitPlanMode`'s plan (2.8c), whole; never on the card.
    Plan(PlanPreview),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanPreview {
    pub markdown: String,
    /// Shortened on the way and not found whole in its plan file.
    pub cut: bool,
}

/// A file edit as changed lines. `added`/`removed` are `None` when the input was cut (2.8a's 16 KB per string), so
/// the card says "cut" instead of a wrong count.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffPreview {
    /// Relative to the folder when inside it.
    pub file: String,
    pub lines: Vec<DiffLine>,
    pub added: Option<u32>,
    pub removed: Option<u32>,
    /// Lines after `lines` that weren't sent ("+12 more lines").
    pub more_lines: u32,
    pub cut: bool,
    /// "New file", "Every occurrence", "Replaces the whole file".
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub op: DiffOp,
    pub text: String,
}

/// `gap`: unchanged lines left out between two changes (its `text` is empty).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffOp {
    Add,
    Del,
    Ctx,
    Gap,
}

/// A shell command, wrapped at token boundaries to `preview::WRAP_COLS` (continuation lines indented two spaces).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandPreview {
    pub lines: Vec<String>,
    pub more_lines: u32,
    pub cut: bool,
}

/// Cay is acting on the desktop for a run (2.7d). Sticky and not interactive; withdrawn when the action ends.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentActingActivity {
    pub run_id: String,
    pub project: String,
    /// "Clicking Save", "Typing into Notepad".
    pub step: String,
    pub phase: AgentActingPhase,
}

/// `waiting`: Cay acts once you've let go of the mouse and keyboard. `acting`: it's acting now.
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
    /// Which agent runs it (2.9b): Claude Code, or an ACP agent.
    pub backend: AgentBackend,
    /// Folder name of `cwd`.
    pub project: String,
    pub cwd: String,
    /// The first prompt, shortened, for the command window's run list.
    pub prompt: String,
    pub status: AgentStatus,
    /// Set once the run has ended.
    pub reason: Option<AgentEndReason>,
    /// What it's doing now ("Bash · git status", 2.8d), while working.
    pub step: Option<String>,
    /// The latest `TodoWrite`'s progress (2.8d), while working.
    pub todos: Option<TodoCount>,
    /// Its permission mode as Claude Code last reported it (or as Cay just set it) (2.8e).
    pub mode: AgentMode,
    /// A mode chosen while no process runs: applies with your next message.
    pub next_mode: Option<AgentMode>,
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

/// When a routine runs by itself, while Cay is running (2.7e). Times are local `HH:MM`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Schedule {
    /// Only by hand.
    Off,
    /// At `time` on each of `days`.
    Days { time: String, days: Vec<Weekday> },
    /// Every `hours` hours (1–24), counted from `from` on the day the schedule was saved.
    Every { hours: u32, from: String },
    /// Two minutes after Cay starts.
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

/// A due time that passed while Cay wasn't running or the PC slept: run once when back, or skip it.
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
    /// Missed (Cay closed, asleep) and set to skip, or more than a day ago.
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
    /// Cay's desktop looks and actions; off passes them in `--disallowedTools`.
    pub desktop_tools: bool,
    /// Cay's Google tools (6c: calendar, tasks, Gmail); off passes them in `--disallowedTools`. Off for routines
    /// saved before 6c.
    #[serde(default)]
    pub google_tools: bool,
    /// The mode its runs start in (2.8e). Routines saved before 2.8e read as Ask.
    #[serde(default = "ask_mode")]
    pub mode: AgentMode,
    /// The agent its runs use (2.9c). Routines saved before 2.9c read as Claude Code.
    #[serde(default)]
    pub backend: AgentBackend,
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

/// Whether an attachment goes to Claude as an image or is passed as a file (2.10b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AttachmentKind {
    Image,
    File,
}

/// An attachment as the transcript and the audit show it: never its content or where it came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAttachment {
    pub name: String,
    pub kind: AttachmentKind,
    /// The original file's size.
    pub bytes: u64,
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
    /// A prompt or follow-up you sent, and what you attached to it (2.10b: names, kinds and sizes only).
    User {
        text: String,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<AgentAttachment>,
    },
    /// Claude's text.
    Text {
        text: String,
    },
    Tool {
        id: String,
        name: String,
        label: String,
        /// An edit's diff, as the approval card shows it (2.10a; built from the input, no file read).
        #[serde(skip_serializing_if = "Option::is_none")]
        preview: Option<ApprovalPreview>,
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
        /// `user`, `remembered`, `timeout`, `runEnded`, or for questions and plans (2.8c) `answered`, `approved`, `feedback`.
        by: &'static str,
    },
    Result {
        status: AgentStatus,
        reason: AgentEndReason,
        turns: u32,
        cost_usd: Option<f64>,
        /// From the message that started this stretch of work to its end, as Cay timed it (2.10a).
        duration_ms: Option<i64>,
    },
    Error {
        message: String,
    },
}

/// `agent:output`.
/// `agent:textDraft` (2.10a): the whole draft so far, at most 10 times a second.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTextDraft {
    pub run_id: String,
    pub text: String,
}

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

/// A `claude.exe` Cay found.
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
    /// The mode a new task starts in (2.8e).
    pub mode: AgentMode,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeHooksStatus {
    pub installed: bool,
    /// Connected by an earlier version (or TopBar): Connect updates it (2.8f).
    pub out_of_date: bool,
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

/// `processes:heavy` and `list_heavy_processes` (Phase 3).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeavyProcessesPayload {
    /// `processes.enabled` in settings.json; the panel shows no section while it's off.
    pub enabled: bool,
    /// Biggest first.
    pub groups: Vec<ProcessGroup>,
}

/// One app: its processes (same exe, plus helpers such as WebView2 it started), summed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessGroup {
    /// Stable across refreshes (the lowercased exe path).
    pub key: String,
    /// The exe's description ("Google Chrome"), else its file name without `.exe`.
    pub name: String,
    pub pids: Vec<ProcessId>,
    /// Private working set, as Task Manager's "Memory" column.
    pub memory_bytes: u64,
    /// PNG data URL.
    pub icon: Option<String>,
    /// Cay never ends it (system, security, the desktop, services, Cay itself).
    pub protected: bool,
}

/// A process, identified by pid *and* creation time so a reused pid is never mistaken for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessId {
    pub pid: u32,
    /// FILETIME ticks as a decimal string (too big for a JS number).
    pub created: String,
}

/// `end_process_group`'s reply (Phase 3b).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndResult {
    /// You chose Cancel in the confirm box: nothing was done.
    pub cancelled: bool,
    /// One per pid asked for (none when cancelled).
    pub outcomes: Vec<EndProcessOutcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndProcessOutcome {
    pub pid: u32,
    pub created: String,
    pub outcome: EndOutcome,
}

/// What happened to one process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EndOutcome {
    /// Cay ended it (`TerminateProcess`).
    Ended,
    /// It exited by itself after the app was asked to close.
    Closed,
    /// It runs as administrator (or is otherwise off limits): Cay can't end it.
    AccessDenied,
    /// The app has a protected member: Cay never ends it.
    Protected,
    /// It had already exited.
    NotFound,
    /// Not the process that was listed (its pid was reused), or not part of that app.
    Changed,
    /// The app is showing a dialog after being asked to close ("Save changes?"): left open for you.
    Prompted,
    /// Windows refused for another reason (logged).
    Failed,
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

// ---- Phase 6: Google Calendar and Tasks ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoogleStatus {
    Disconnected,
    /// The browser is open on Google's sign-in.
    Connecting,
    Connected,
    /// Google no longer accepts the saved sign-in (revoked, or a password change): Connect again.
    Reconnect,
}

/// `google:state` and `google_state`. Never carries a token.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleState {
    pub status: GoogleStatus,
    /// The account's primary calendar id (its address), once a fetch has seen it.
    pub account: Option<String>,
    /// What was granted: Google's consent screen lets you untick a scope.
    pub calendar: bool,
    pub tasks: bool,
    /// Gmail for agent runs (6c): reading and drafts were granted.
    pub gmail: bool,
    pub last_sync_ms: Option<i64>,
    /// Why the last refresh or sign-in failed (offline, Google's error); the last data is kept.
    pub error: Option<String>,
    /// This build (or settings.json) has a Google client to sign in with.
    pub configured: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventPhase {
    Later,
    /// Starts within `google.soonMinutes`.
    Soon,
    /// In progress.
    Now,
}

/// An upcoming event. Its links stay in Rust (`calendar_open` / `calendar_join` take the id).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    /// Google's event id (one per recurring instance).
    pub id: String,
    pub title: String,
    pub start_ms: i64,
    pub end_ms: i64,
    /// The calendar's name.
    pub calendar: String,
    /// The calendar's colour in Google (`#rrggbb`).
    pub color: Option<String>,
    pub has_join: bool,
    pub phase: EventPhase,
}

/// An open task due today or overdue. Tasks have dates, not times.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleTask {
    pub id: String,
    pub title: String,
    /// `YYYY-MM-DD`.
    pub due: String,
    pub overdue: bool,
    /// The task list's name.
    pub list: String,
}

/// `calendar:upcoming` and `calendar_upcoming`: the next 24 h of events (soonest first) and due tasks.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarUpcoming {
    pub events: Vec<CalendarEvent>,
    pub tasks: Vec<GoogleTask>,
    /// The pill's calendar icon shows: an event soon or in progress, or a task due.
    pub pending: bool,
}

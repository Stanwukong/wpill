//! The client side of the Agent Client Protocol (2.9b, D47/D48): JSON-RPC 2.0, one message per line, to an agent
//! that runs as Cay's child (`opencode acp`, `codex-acp`, `goose acp`). Pure; unit-tested with messages captured from
//! OpenCode 1.19.5.
//!
//! It turns what the agent sends into the same `StreamEvent`s a Claude Code run produces, so `lifecycle.rs` drives
//! both, and turns Cay's prompts, mode changes and permission answers into lines for the agent's stdin. A run keeps
//! Claude Code's shape: one process per stretch of work, a follow-up in a new process that resumes the session
//! (`session/load`) when the agent offers it.
//!
//! Everything the agent sends is untrusted: unknown messages are ignored, requests Cay didn't offer (`fs/*`,
//! `terminal/*`) are refused, and nothing it says is ever taken as your answer.

use std::collections::{HashMap, VecDeque};

use serde_json::{json, Map, Value};

use super::stream::{cut, StreamEvent, TurnResult, MAX_TEXT, MAX_TOOL_RESULT};

pub const PROTOCOL_VERSION: u64 = 1;
/// JSON-RPC's "method not found".
const NOT_SUPPORTED: i64 = -32601;

/// What happens next, for `agent/mod.rs` to carry out.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// A line for the agent's stdin (newline included).
    Write(Vec<u8>),
    Event(StreamEvent),
    /// The agent asks to run a tool: show a card, then `answer`.
    Permission {
        rpc_id: Value,
        tool_call_id: String,
        /// Claude Code's name for it (`Bash`, `Edit`, …), so cards and remember keys read the same.
        tool: String,
        input: Value,
    },
    /// An edit, a shell command, a delete, a move or a fetch finished without ever being asked about. Only reported
    /// while `ask_mode` is set (D48's watchdog): the run is stopped.
    Unasked {
        tool: String,
        summary: String,
    },
    /// A line for the transcript (the agent couldn't resume, a mode it doesn't have).
    Notice(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// `initialize` sent.
    Starting,
    /// `session/new` or `session/load` sent.
    Opening,
    Ready,
    /// The agent failed to start or open a session; everything after is ignored.
    Broken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Call {
    Initialize,
    New,
    Load,
    Prompt,
    /// `critical`: the run's mode; refused means the run stops rather than go on in another mode.
    Config {
        critical: bool,
    },
}

#[derive(Debug, Default)]
struct ToolCall {
    /// The agent's tool name from the first `tool_call` (OpenCode: `bash`, `edit`, …).
    title: String,
    kind: String,
    input: Map<String, Value>,
    shown: bool,
    asked: bool,
    done: bool,
}

/// What to ask for once the session is open.
#[derive(Debug, Clone, Default)]
pub struct Setup {
    pub cwd: String,
    /// `session/new`'s `mcpServers` (Cay's own server, `cay.exe mcp`).
    pub mcp_servers: Value,
    /// The session to resume, for a follow-up in a new process.
    pub resume: Option<String>,
    /// Config options to set once open (`mode` → `plan`, `model` → …), applied only if the agent offers them.
    pub config: Vec<(String, String)>,
    /// Ask mode: report steps that ran unasked.
    pub ask_mode: bool,
}

#[derive(Debug)]
pub struct Client {
    setup: Setup,
    phase: Phase,
    next_id: u64,
    calls: HashMap<u64, Call>,
    session: Option<String>,
    can_load: bool,
    queued: VecDeque<String>,
    prompting: bool,
    /// Config changes sent and not answered yet: the first prompt waits for them.
    configuring: u32,
    text: String,
    tools: HashMap<String, ToolCall>,
    /// Permission requests waiting for an answer: JSON-RPC id → their options.
    asking: HashMap<String, Vec<(String, String)>>,
    model: Option<String>,
    cost_usd: Option<f64>,
}

fn line(message: Value) -> Vec<u8> {
    let mut bytes = message.to_string().into_bytes();
    bytes.push(b'\n');
    bytes
}

fn key(id: &Value) -> String {
    id.to_string()
}

impl Client {
    /// A client and the `initialize` line to send first.
    pub fn new(setup: Setup) -> (Self, Vec<u8>) {
        let mut client = Client {
            setup,
            phase: Phase::Starting,
            next_id: 0,
            calls: HashMap::new(),
            session: None,
            can_load: false,
            queued: VecDeque::new(),
            prompting: false,
            configuring: 0,
            text: String::new(),
            tools: HashMap::new(),
            asking: HashMap::new(),
            model: None,
            cost_usd: None,
        };
        // Cay offers no file or terminal access of its own: the agent uses its own tools, under its own permissions.
        let first = client.request(
            Call::Initialize,
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false },
            }),
        );
        (client, first)
    }

    fn request(&mut self, call: Call, method: &str, params: Value) -> Vec<u8> {
        self.next_id += 1;
        self.calls.insert(self.next_id, call);
        line(json!({ "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params }))
    }

    /// A message from you. Sent now if the session is open and idle, else after the current turn.
    pub fn prompt(&mut self, text: &str) -> Vec<Out> {
        self.queued.push_back(text.to_string());
        self.next_prompt()
    }

    fn next_prompt(&mut self) -> Vec<Out> {
        if self.phase != Phase::Ready || self.prompting || self.configuring > 0 {
            return Vec::new();
        }
        let (Some(text), Some(session)) = (self.queued.pop_front(), self.session.clone()) else {
            return Vec::new();
        };
        self.prompting = true;
        let bytes = self.request(
            Call::Prompt,
            "session/prompt",
            json!({ "sessionId": session, "prompt": [{ "type": "text", "text": text }] }),
        );
        vec![Out::Write(bytes)]
    }

    /// Sets a config option (`mode`, `model`) on the open session.
    pub fn set_config(&mut self, id: &str, value: &str) -> Vec<Out> {
        let Some(session) = self.session.clone() else {
            return Vec::new();
        };
        self.configuring += 1;
        let critical = is_mode(id);
        let bytes = self.request(
            Call::Config { critical },
            "session/set_config_option",
            json!({ "sessionId": session, "configId": id, "value": value }),
        );
        vec![Out::Write(bytes)]
    }

    /// Your answer to a permission request: allow picks the agent's "once" option, deny its "reject once". Cay never
    /// picks an "always" allow; with no "once" option an allow is sent as a reject.
    pub fn answer(&mut self, rpc_id: &Value, allow: bool) -> Option<Vec<u8>> {
        let options = self.asking.remove(&key(rpc_id))?;
        let pick = |kind: &str| {
            options
                .iter()
                .find(|(_, k)| k == kind)
                .map(|(id, _)| id.clone())
        };
        let chosen = if allow {
            pick("allow_once")
                .or_else(|| pick("reject_once"))
                .or_else(|| pick("reject_always"))
        } else {
            pick("reject_once").or_else(|| pick("reject_always"))
        };
        let outcome = match chosen {
            Some(option_id) => json!({ "outcome": "selected", "optionId": option_id }),
            None => json!({ "outcome": "cancelled" }),
        };
        Some(line(
            json!({ "jsonrpc": "2.0", "id": rpc_id, "result": { "outcome": outcome } }),
        ))
    }

    /// The run is ending: every open permission request is cancelled.
    pub fn cancel_all(&mut self) -> Vec<Vec<u8>> {
        self.asking
            .drain()
            .filter_map(|(id, _)| serde_json::from_str::<Value>(&id).ok())
            .map(|id| line(json!({ "jsonrpc": "2.0", "id": id, "result": { "outcome": { "outcome": "cancelled" } } })))
            .collect()
    }

    /// One line from the agent's stdout.
    pub fn on_line(&mut self, bytes: &[u8]) -> Vec<Out> {
        let Ok(message) = serde_json::from_slice::<Value>(bytes) else {
            return Vec::new();
        };
        if self.phase == Phase::Broken {
            return Vec::new();
        }
        let method = message.get("method").and_then(Value::as_str);
        let id = message.get("id").filter(|v| !v.is_null()).cloned();
        match (method, id) {
            (Some(method), Some(id)) => self.on_request(method, id, &message["params"]),
            (Some("session/update"), None) => self.on_update(&message["params"]),
            (Some(_), None) => Vec::new(),
            (None, Some(id)) => self.on_response(&id, &message),
            (None, None) => Vec::new(),
        }
    }

    fn on_response(&mut self, id: &Value, message: &Value) -> Vec<Out> {
        let Some(call) = id.as_u64().and_then(|n| self.calls.remove(&n)) else {
            return Vec::new();
        };
        let error = message.get("error").map(|e| {
            e.get("message")
                .and_then(Value::as_str)
                .unwrap_or("the agent reported an error")
                .to_string()
        });
        let result = &message["result"];
        match (call, error) {
            (Call::Initialize, None) => {
                self.can_load = result
                    .pointer("/agentCapabilities/loadSession")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.open()
            }
            (Call::New | Call::Load, None) => {
                if call == Call::New {
                    match result.get("sessionId").and_then(Value::as_str) {
                        Some(id) => self.session = Some(id.to_string()),
                        None => return self.broken("The agent didn't open a session.".into()),
                    }
                }
                self.phase = Phase::Ready;
                let mut out = self.configure(result.get("configOptions"));
                if self.phase == Phase::Broken {
                    return out;
                }
                out.push(Out::Event(StreamEvent::Init {
                    session_id: self.session.clone().unwrap_or_default(),
                    model: self.model.clone(),
                }));
                out.extend(self.next_prompt());
                out
            }
            (Call::Prompt, error) => {
                self.prompting = false;
                let mut out = self.flush_text();
                let stop = result
                    .get("stopReason")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let (subtype, is_error) = match (stop, &error) {
                    (_, Some(_)) => ("error_during_execution", true),
                    ("end_turn", None) => ("success", false),
                    ("max_turn_requests", None) => ("error_max_turns", true),
                    // Cay never cancels a turn (Stop ends the process), so this is the agent's own answer to a
                    // declined step: Codex's "reject once" option cancels the turn.
                    ("cancelled", None) => {
                        out.push(Out::Notice(
                            "The agent ended this turn after a step was declined.".into(),
                        ));
                        ("success", false)
                    }
                    ("refusal", None) => ("error_refusal", true),
                    ("max_tokens", None) => ("error_max_tokens", true),
                    _ => ("success", false),
                };
                let errors = match (&error, stop) {
                    (Some(e), _) => vec![e.clone()],
                    (None, "refusal") => vec!["The model refused to continue.".into()],
                    (None, "max_tokens") => vec!["The model hit its output limit.".into()],
                    _ => Vec::new(),
                };
                out.push(Out::Event(StreamEvent::Result(TurnResult {
                    subtype: subtype.into(),
                    is_error,
                    num_turns: 1,
                    total_cost_usd: self.cost_usd,
                    result: None,
                    errors,
                })));
                out.extend(self.next_prompt());
                out
            }
            (Call::Config { .. }, None) => {
                self.configuring = self.configuring.saturating_sub(1);
                self.configure_model(result.get("configOptions"));
                self.next_prompt()
            }
            (Call::Config { critical: true }, Some(e)) => {
                self.broken(format!("The agent didn't switch to this run's mode: {e}"))
            }
            (Call::Config { critical: false }, Some(e)) => {
                self.configuring = self.configuring.saturating_sub(1);
                let mut out = vec![Out::Notice(format!(
                    "The agent didn't change a setting: {e}"
                ))];
                out.extend(self.next_prompt());
                out
            }
            (Call::Initialize | Call::New | Call::Load, Some(e)) => {
                self.broken(format!("The agent couldn't start: {e}"))
            }
        }
    }

    /// After `initialize`: resume the session if asked and the agent can, else open a new one.
    fn open(&mut self) -> Vec<Out> {
        self.phase = Phase::Opening;
        let mut out = Vec::new();
        let params = |extra: Option<&str>| {
            let mut p = json!({ "cwd": self.setup.cwd, "mcpServers": self.setup.mcp_servers });
            if let Some(session) = extra {
                p["sessionId"] = json!(session);
            }
            p
        };
        match self.setup.resume.clone() {
            Some(session) if self.can_load => {
                self.session = Some(session.clone());
                let p = params(Some(&session));
                out.push(Out::Write(self.request(Call::Load, "session/load", p)));
            }
            resume => {
                if resume.is_some() {
                    out.push(Out::Notice(
                        "This agent can't resume a conversation, so this message starts a new one."
                            .into(),
                    ));
                }
                let p = params(None);
                out.push(Out::Write(self.request(Call::New, "session/new", p)));
            }
        }
        out
    }

    /// The requested config options the agent offers, as `session/set_config_option` calls.
    fn configure(&mut self, options: Option<&Value>) -> Vec<Out> {
        self.configure_model(options);
        let offered = options
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for (id, value) in self.setup.config.clone() {
            let option = offered.iter().find(|o| o["id"] == json!(id));
            let current = option.and_then(|o| o["currentValue"].as_str());
            let has = option
                .and_then(|o| o["options"].as_array())
                .is_some_and(|vs| vs.iter().any(|v| v["value"] == json!(value)));
            if current == Some(value.as_str()) {
                continue;
            }
            if has {
                out.extend(self.set_config(&id, &value));
            } else if is_mode(&id) {
                return self.broken(format!(
                    "The agent has no {id} \"{value}\" for this run's mode."
                ));
            } else {
                out.push(Out::Notice(format!(
                    "The agent has no {id} \"{value}\"; it keeps its own."
                )));
            }
        }
        out
    }

    fn configure_model(&mut self, options: Option<&Value>) {
        let model = options
            .and_then(Value::as_array)
            .and_then(|os| os.iter().find(|o| o["id"] == json!("model")))
            .and_then(|o| o["currentValue"].as_str());
        if let Some(model) = model {
            self.model = Some(model.to_string());
        }
    }

    fn broken(&mut self, why: String) -> Vec<Out> {
        self.phase = Phase::Broken;
        let mut out = vec![Out::Event(StreamEvent::Result(TurnResult {
            subtype: "error_during_execution".into(),
            is_error: true,
            num_turns: 0,
            total_cost_usd: None,
            result: None,
            errors: vec![why],
        }))];
        // Every queued message gets its own result, so the run can finish.
        let left = self.queued.len();
        self.queued.clear();
        for _ in 0..left {
            out.push(out[0].clone());
        }
        out
    }

    fn on_request(&mut self, method: &str, id: Value, params: &Value) -> Vec<Out> {
        if method != "session/request_permission" {
            return vec![Out::Write(line(json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": NOT_SUPPORTED, "message": format!("Cay doesn't offer {method}") },
            })))];
        }
        let call = &params["toolCall"];
        let call_id = call["toolCallId"].as_str().unwrap_or("").to_string();
        let mut out = self.flush_text();
        out.extend(self.track(call));
        let options: Vec<(String, String)> = params["options"]
            .as_array()
            .map(|os| {
                os.iter()
                    .filter_map(|o| {
                        Some((
                            o["optionId"].as_str()?.to_string(),
                            o["kind"].as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.asking.insert(key(&id), options);
        let tool = self.tools.entry(call_id.clone()).or_default();
        tool.asked = true;
        let (name, input) = shape(&tool.title, &tool.kind, &tool.input);
        out.push(Out::Permission {
            rpc_id: id,
            tool_call_id: call_id,
            tool: name,
            input: Value::Object(input),
        });
        out
    }

    fn on_update(&mut self, params: &Value) -> Vec<Out> {
        let update = &params["update"];
        let kind = update["sessionUpdate"].as_str().unwrap_or("");
        // Messages replayed by `session/load` were shown when they happened.
        if self.phase == Phase::Opening {
            return Vec::new();
        }
        match kind {
            "agent_message_chunk" => {
                if let Some(text) = update.pointer("/content/text").and_then(Value::as_str) {
                    if self.text.len() < MAX_TEXT * 2 {
                        self.text.push_str(text);
                    }
                }
                Vec::new()
            }
            "tool_call" | "tool_call_update" => {
                let mut out = self.flush_text();
                out.extend(self.track(update));
                out
            }
            "usage_update" => {
                if let Some(amount) = update.pointer("/cost/amount").and_then(Value::as_f64) {
                    self.cost_usd = Some(amount);
                }
                Vec::new()
            }
            "config_option_update" => {
                self.configure_model(update.get("configOptions"));
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// A `tool_call` or `tool_call_update`: shown once its input is known, its result once it ends.
    fn track(&mut self, call: &Value) -> Vec<Out> {
        let Some(id) = call["toolCallId"].as_str() else {
            return Vec::new();
        };
        let ask_mode = self.setup.ask_mode;
        let tool = self.tools.entry(id.to_string()).or_default();
        if tool.title.is_empty() {
            if let Some(title) = call["title"].as_str() {
                tool.title = title.to_string();
            }
        }
        if let Some(kind) = call["kind"].as_str() {
            tool.kind = kind.to_string();
        }
        if let Some(input) = call["rawInput"].as_object() {
            for (k, v) in input {
                tool.input.insert(k.clone(), v.clone());
            }
        }
        let status = call["status"].as_str().unwrap_or("");
        let ended = matches!(status, "completed" | "failed");
        let known = tool.input.keys().any(|k| k != "cwd");
        let mut out = Vec::new();
        let (name, input) = shape(&tool.title, &tool.kind, &tool.input);
        if !tool.shown && (known || ended || status == "in_progress") {
            tool.shown = true;
            out.push(Out::Event(StreamEvent::Tool {
                id: id.to_string(),
                name: name.clone(),
                input: Value::Object(input.clone()),
            }));
        }
        if ended && !tool.done {
            tool.done = true;
            let text = content_text(&call["content"]);
            // Cay's own tools are gated by Cay, and an agent's review of a step isn't the step.
            let gated_by_cay = name.starts_with("mcp__cay__") || name == REVIEW;
            if ask_mode
                && status == "completed"
                && !tool.asked
                && is_risky(&tool.kind)
                && !gated_by_cay
            {
                let summary = super::describe::summary(&name, &Value::Object(input), "");
                out.push(Out::Unasked {
                    tool: name.clone(),
                    summary,
                });
            }
            out.push(Out::Event(StreamEvent::ToolResult {
                id: id.to_string(),
                is_error: status == "failed",
                text: cut(&text, MAX_TOOL_RESULT),
            }));
        }
        out
    }

    fn flush_text(&mut self) -> Vec<Out> {
        let text = std::mem::take(&mut self.text);
        if text.trim().is_empty() {
            return Vec::new();
        }
        vec![Out::Event(StreamEvent::Text(cut(text.trim(), MAX_TEXT)))]
    }
}

/// Config options that decide what the agent may do without asking (OpenCode's `mode`, Codex's `mode` and
/// `collaboration_mode`).
fn is_mode(id: &str) -> bool {
    matches!(id, "mode" | "collaboration_mode")
}

/// Steps Ask mode promises to ask about (the watchdog's kinds).
fn is_risky(kind: &str) -> bool {
    matches!(kind, "edit" | "delete" | "move" | "execute" | "fetch")
}

/// Claude Code's name for an agent's tool: by the agent's own tool name when Cay knows it, else by ACP's kind.
pub fn claude_name(title: &str, kind: &str) -> String {
    // OpenCode names MCP tools `<server>_<tool>`; Cay's own read as Claude Code names them, so grants and cards match.
    if let Some(tool) = title.strip_prefix("cay_") {
        let ours = super::tools::NAMES
            .iter()
            .chain(super::actions::NAMES)
            .chain(crate::google::tools::NAMES)
            .any(|n| *n == tool);
        if ours {
            return format!("mcp__{}__{tool}", super::mcp::SERVER_NAME);
        }
    }
    let known = match title {
        "bash" | "shell" | "execute" => Some("Bash"),
        "edit" => Some("Edit"),
        "multiedit" => Some("MultiEdit"),
        "write" => Some("Write"),
        "read" => Some("Read"),
        "glob" => Some("Glob"),
        "grep" => Some("Grep"),
        "list" => Some("LS"),
        "webfetch" => Some("WebFetch"),
        "websearch" => Some("WebSearch"),
        "todowrite" => Some("TodoWrite"),
        "task" => Some("Task"),
        "patch" | "apply_patch" => Some("apply_patch"),
        _ => None,
    };
    if let Some(name) = known {
        return name.into();
    }
    match kind {
        "execute" => "Bash".into(),
        "edit" => "Edit".into(),
        "read" => "Read".into(),
        "search" => "Grep".into(),
        "fetch" => "WebFetch".into(),
        "delete" => "Delete".into(),
        "move" => "Move".into(),
        _ if !title.is_empty() && title.len() <= 64 && !title.contains(' ') => title.into(),
        _ => "Tool".into(),
    }
}

/// How Cay shows an agent's own check of a step (Codex's Auto review).
const REVIEW: &str = "Review";

/// The tool as Claude Code would name it, and its arguments. Codex reports an MCP call as `{server, tool, arguments}`
/// (read as `mcp__<server>__<tool>`, so Cay's grants and cards match) and its Auto review of a step as a call with a
/// `reviewId` (shown as `Review`, naming the reviewed tool).
fn shape(title: &str, kind: &str, input: &Map<String, Value>) -> (String, Map<String, Value>) {
    let text = |key: &str| input.get(key).and_then(Value::as_str);
    if let (Some(server), Some(tool)) = (text("server"), text("tool")) {
        let args = input
            .get("arguments")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        return (format!("mcp__{server}__{tool}"), args);
    }
    if input.contains_key("reviewId") {
        let action = input.get("action");
        let what = action
            .and_then(|a| a.get("toolName"))
            .filter(|v| !v.is_null())
            .or_else(|| action.and_then(|a| a.get("type")))
            .and_then(Value::as_str);
        let mut shown = Map::new();
        if let Some(what) = what {
            shown.insert("description".into(), json!(what));
        }
        return (REVIEW.into(), shown);
    }
    (claude_name(title, kind), claude_input(input))
}

/// The agent's arguments under Claude Code's names (`filePath` → `file_path`, …), without the `cwd` ACP adds.
fn claude_input(input: &Map<String, Value>) -> Map<String, Value> {
    let renamed = crate::sessions::claude_keys(Value::Object(input.clone()));
    let mut map = renamed.as_object().cloned().unwrap_or_default();
    map.remove("cwd");
    map
}

fn content_text(content: &Value) -> String {
    content
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|c| c.pointer("/content/text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "ses_e5a618c9bffefXPB5GVYjmK50Y";

    fn setup() -> Setup {
        Setup {
            cwd: r"C:\work".into(),
            mcp_servers: json!([{ "name": "cay", "command": r"C:\cay.exe", "args": ["mcp"], "env": [] }]),
            resume: None,
            config: vec![],
            ask_mode: true,
        }
    }

    fn feed(c: &mut Client, message: Value) -> Vec<Out> {
        c.on_line(message.to_string().as_bytes())
    }

    fn written(out: &[Out]) -> Vec<Value> {
        out.iter()
            .filter_map(|o| match o {
                Out::Write(b) => serde_json::from_slice(b).ok(),
                _ => None,
            })
            .collect()
    }

    fn events(out: &[Out]) -> Vec<StreamEvent> {
        out.iter()
            .filter_map(|o| match o {
                Out::Event(e) => Some(e.clone()),
                _ => None,
            })
            .collect()
    }

    /// Initialized and a session open, as OpenCode 1.19.5 answered.
    fn open(setup: Setup) -> (Client, Vec<Out>) {
        let (mut c, first) = Client::new(setup);
        let init: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(init["method"], json!("initialize"));
        assert_eq!(
            init["params"]["clientCapabilities"]["terminal"],
            json!(false)
        );
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": 1, "result": {
            "protocolVersion": 1, "agentCapabilities": { "loadSession": true } } }),
        );
        let new = &written(&out)[0];
        assert_eq!(new["method"], json!("session/new"));
        assert_eq!(new["params"]["mcpServers"][0]["name"], json!("cay"));
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": 2, "result": { "sessionId": SID, "configOptions": [
            { "id": "model", "currentValue": "opencode/deepseek-v4.1-flash-free",
              "options": [{ "value": "opencode/deepseek-v4.1-flash-free" }, { "value": "opencode/big-pickle" }] },
            { "id": "mode", "currentValue": "build", "options": [{ "value": "build" }, { "value": "plan" }] }
        ] } }),
        );
        (c, out)
    }

    #[test]
    fn opens_a_session_then_sends_the_prompt() {
        let (mut c, out) = open(setup());
        assert_eq!(
            events(&out),
            vec![StreamEvent::Init {
                session_id: SID.into(),
                model: Some("opencode/deepseek-v4.1-flash-free".into())
            }]
        );
        let out = c.prompt("make hello.txt");
        let p = &written(&out)[0];
        assert_eq!(p["method"], json!("session/prompt"));
        assert_eq!(p["params"]["prompt"][0]["text"], json!("make hello.txt"));
        // A second message waits for the first turn.
        assert!(c.prompt("then this").is_empty());
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": p["id"], "result": { "stopReason": "end_turn" } }),
        );
        assert!(
            matches!(&events(&out)[0], StreamEvent::Result(r) if r.subtype == "success" && !r.is_error)
        );
        assert_eq!(
            written(&out)[0]["params"]["prompt"][0]["text"],
            json!("then this")
        );
    }

    #[test]
    fn a_prompt_before_the_session_is_open_waits() {
        let (mut c, _) = Client::new(setup());
        assert!(c.prompt("early").is_empty());
        feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": 1, "result": { "agentCapabilities": {} } }),
        );
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": 2, "result": { "sessionId": SID } }),
        );
        assert_eq!(
            written(&out)[0]["params"]["prompt"][0]["text"],
            json!("early")
        );
    }

    #[test]
    fn a_shell_step_asks_and_takes_the_answer() {
        let (mut c, _) = open(setup());
        c.prompt("go");
        feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": SID,
            "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "I'll create " } } } }),
        );
        feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": SID,
            "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "the file." } } } }),
        );
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": SID,
            "update": { "sessionUpdate": "tool_call", "toolCallId": "call_1", "title": "bash", "kind": "execute",
                        "status": "pending", "rawInput": { "cwd": "C:\\work" } } } }),
        );
        // The text so far is one entry; the tool waits until its input is known.
        assert_eq!(
            events(&out),
            vec![StreamEvent::Text("I'll create the file.".into())]
        );
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": SID,
            "update": { "sessionUpdate": "tool_call_update", "toolCallId": "call_1", "status": "in_progress",
                        "kind": "execute", "title": "echo hi > hello.txt",
                        "rawInput": { "command": "echo hi > hello.txt", "cwd": "C:\\work" } } } }),
        );
        assert_eq!(
            events(&out),
            vec![StreamEvent::Tool {
                id: "call_1".into(),
                name: "Bash".into(),
                input: json!({ "command": "echo hi > hello.txt" })
            }]
        );
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": 0, "method": "session/request_permission", "params": {
            "sessionId": SID, "toolCall": { "toolCallId": "call_1", "title": "echo hi > hello.txt", "kind": "execute",
            "status": "pending", "rawInput": { "command": "echo hi > hello.txt" } },
            "options": [{ "optionId": "once", "kind": "allow_once", "name": "Allow once" },
                        { "optionId": "always", "kind": "allow_always", "name": "Always allow" },
                        { "optionId": "reject", "kind": "reject_once", "name": "Reject" }] } }),
        );
        assert_eq!(
            out,
            vec![Out::Permission {
                rpc_id: json!(0),
                tool_call_id: "call_1".into(),
                tool: "Bash".into(),
                input: json!({ "command": "echo hi > hello.txt" }),
            }]
        );
        let reply: Value = serde_json::from_slice(&c.answer(&json!(0), true).unwrap()).unwrap();
        assert_eq!(
            reply,
            json!({ "jsonrpc": "2.0", "id": 0, "result": { "outcome": { "outcome": "selected", "optionId": "once" } } })
        );
        // Answered once only.
        assert_eq!(c.answer(&json!(0), true), None);
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": SID,
            "update": { "sessionUpdate": "tool_call_update", "toolCallId": "call_1", "status": "completed",
                        "content": [{ "type": "content", "content": { "type": "text", "text": "hi" } }] } } }),
        );
        // Asked about, so no watchdog report.
        assert_eq!(
            out,
            vec![Out::Event(StreamEvent::ToolResult {
                id: "call_1".into(),
                is_error: false,
                text: "hi".into()
            })]
        );
    }

    #[test]
    fn deny_never_picks_an_always_option() {
        let (mut c, _) = open(setup());
        feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": "p1", "method": "session/request_permission", "params": {
            "sessionId": SID, "toolCall": { "toolCallId": "c", "kind": "edit", "title": "edit",
            "rawInput": { "filePath": "a.rs", "oldString": "x", "newString": "y" } },
            "options": [{ "optionId": "always", "kind": "allow_always" }, { "optionId": "no", "kind": "reject_always" }] } }),
        );
        let reply: Value = serde_json::from_slice(&c.answer(&json!("p1"), false).unwrap()).unwrap();
        assert_eq!(reply["result"]["outcome"]["optionId"], json!("no"));
        // An allow with only "always" on offer becomes a reject: Cay never widens the agent's rules.
        feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": "p2", "method": "session/request_permission", "params": {
            "sessionId": SID, "toolCall": { "toolCallId": "d", "kind": "edit" },
            "options": [{ "optionId": "always", "kind": "allow_always" }, { "optionId": "no", "kind": "reject_once" }] } }),
        );
        let reply: Value = serde_json::from_slice(&c.answer(&json!("p2"), true).unwrap()).unwrap();
        assert_eq!(reply["result"]["outcome"]["optionId"], json!("no"));
    }

    #[test]
    fn edits_read_like_claude_codes() {
        let (mut c, _) = open(setup());
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": 5, "method": "session/request_permission", "params": {
            "sessionId": SID, "toolCall": { "toolCallId": "e", "kind": "edit", "title": "edit",
            "rawInput": { "filePath": "C:\\work\\a.rs", "oldString": "x", "newString": "y" } },
            "options": [{ "optionId": "once", "kind": "allow_once" }] } }),
        );
        assert!(out.contains(&Out::Permission {
            rpc_id: json!(5),
            tool_call_id: "e".into(),
            tool: "Edit".into(),
            input: json!({ "file_path": "C:\\work\\a.rs", "old_string": "x", "new_string": "y" }),
        }));
    }

    #[test]
    fn the_watchdog_reports_unasked_steps_in_ask_mode_only() {
        let step = json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": SID,
            "update": { "sessionUpdate": "tool_call", "toolCallId": "w", "title": "bash", "kind": "execute",
                        "status": "completed", "rawInput": { "command": "rm notes.txt" } } } });
        let (mut ask, _) = open(setup());
        let out = feed(&mut ask, step.clone());
        assert!(out.contains(&Out::Unasked {
            tool: "Bash".into(),
            summary: "rm notes.txt".into()
        }));
        let (mut auto, _) = open(Setup {
            ask_mode: false,
            ..setup()
        });
        assert!(!feed(&mut auto, step)
            .iter()
            .any(|o| matches!(o, Out::Unasked { .. })));
        // Reads never count.
        let (mut ask, _) = open(setup());
        let read = json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": SID,
            "update": { "sessionUpdate": "tool_call", "toolCallId": "r", "title": "read", "kind": "read",
                        "status": "completed", "rawInput": { "filePath": "a.rs" } } } });
        assert!(!feed(&mut ask, read)
            .iter()
            .any(|o| matches!(o, Out::Unasked { .. })));
    }

    #[test]
    fn requests_cay_doesnt_offer_are_refused() {
        let (mut c, _) = open(setup());
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": 9, "method": "fs/write_text_file",
            "params": { "path": "C:\\x", "content": "y" } }),
        );
        assert_eq!(written(&out)[0]["error"]["code"], json!(NOT_SUPPORTED));
        assert!(feed(&mut c, json!("garbage")).is_empty());
        assert!(c.on_line(b"not json").is_empty());
    }

    #[test]
    fn a_follow_up_resumes_and_skips_the_replay() {
        let (mut c, first) = Client::new(Setup {
            resume: Some(SID.into()),
            ..setup()
        });
        assert!(!first.is_empty());
        c.prompt("and now?");
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": 1, "result": { "agentCapabilities": { "loadSession": true } } }),
        );
        let load = &written(&out)[0];
        assert_eq!(
            (load["method"].clone(), load["params"]["sessionId"].clone()),
            (json!("session/load"), json!(SID))
        );
        // The replay of the old conversation isn't shown again.
        let replay = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": SID,
            "update": { "sessionUpdate": "tool_call", "toolCallId": "old", "title": "bash", "kind": "execute",
                        "status": "completed", "rawInput": { "command": "ls" } } } }),
        );
        assert!(replay.is_empty());
        let out = feed(&mut c, json!({ "jsonrpc": "2.0", "id": 2, "result": {} }));
        assert!(
            matches!(&events(&out)[0], StreamEvent::Init { session_id, .. } if session_id == SID)
        );
        assert_eq!(
            written(&out)[0]["params"]["prompt"][0]["text"],
            json!("and now?")
        );

        // Without loadSession: a new session, and a note saying so.
        let (mut c, _) = Client::new(Setup {
            resume: Some(SID.into()),
            ..setup()
        });
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": 1, "result": { "agentCapabilities": {} } }),
        );
        assert!(matches!(&out[0], Out::Notice(_)));
        assert_eq!(written(&out)[0]["method"], json!("session/new"));
    }

    #[test]
    fn plan_mode_and_a_model_are_set_when_offered() {
        let config = vec![
            ("mode".to_string(), "plan".to_string()),
            ("model".to_string(), "opencode/big-pickle".to_string()),
        ];
        let (_, out) = open(Setup { config, ..setup() });
        let sets = written(&out);
        assert_eq!(sets.len(), 2);
        assert_eq!(
            (
                sets[0]["method"].clone(),
                sets[0]["params"]["value"].clone()
            ),
            (json!("session/set_config_option"), json!("plan"))
        );
        assert_eq!(sets[1]["params"]["configId"], json!("model"));
        // A mode the agent doesn't have ends the run rather than running in another mode; a model is only a note.
        let config = vec![("mode".to_string(), "chat".to_string())];
        let (_, out) = open(Setup { config, ..setup() });
        assert!(matches!(&events(&out)[..], [StreamEvent::Result(r)] if r.is_error));
        let config = vec![("model".to_string(), "gpt-9".to_string())];
        let (_, out) = open(Setup { config, ..setup() });
        assert!(out
            .iter()
            .any(|o| matches!(o, Out::Notice(n) if n.contains("gpt-9"))));
    }

    #[test]
    fn the_first_prompt_waits_for_the_mode() {
        let config = vec![("mode".to_string(), "plan".to_string())];
        let (mut c, out) = open(Setup { config, ..setup() });
        let set = written(&out)[0].clone();
        assert_eq!(set["params"]["value"], json!("plan"));
        // Asked before the agent confirmed the mode: nothing is sent yet.
        assert!(c.prompt("go").is_empty());
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": set["id"], "result": { "configOptions": [] } }),
        );
        assert_eq!(written(&out)[0]["method"], json!("session/prompt"));

        // Refused: the run ends with the reason, and the prompt is never sent.
        let config = vec![("mode".to_string(), "plan".to_string())];
        let (mut c, out) = open(Setup { config, ..setup() });
        let set = written(&out)[0].clone();
        c.prompt("go");
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": set["id"], "error": { "code": -1, "message": "nope" } }),
        );
        assert!(written(&out).is_empty());
        assert!(
            matches!(&events(&out)[0], StreamEvent::Result(r) if r.is_error && r.errors[0].contains("mode"))
        );
    }

    #[test]
    fn failures_end_the_turn_with_the_reason() {
        let (mut c, _) = Client::new(setup());
        c.prompt("hi");
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32000, "message": "Authentication required" } }),
        );
        match &events(&out)[0] {
            StreamEvent::Result(r) => {
                assert!(r.is_error);
                assert!(r.errors[0].contains("Authentication required"));
            }
            other => panic!("{other:?}"),
        }
        let (mut c, _) = open(setup());
        let p = written(&c.prompt("x"))[0].clone();
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": p["id"], "result": { "stopReason": "max_turn_requests" } }),
        );
        assert!(
            matches!(&events(&out)[0], StreamEvent::Result(r) if r.subtype == "error_max_turns")
        );
    }

    #[test]
    fn codex_mcp_calls_and_reviews_read_clearly() {
        let (mut c, _) = open(setup());
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": SID,
            "update": { "sessionUpdate": "tool_call", "toolCallId": "m", "kind": "execute", "status": "completed",
                        "rawInput": { "server": "cay", "tool": "list_windows", "arguments": {} } } } }),
        );
        assert!(events(&out).contains(&StreamEvent::Tool {
            id: "m".into(),
            name: "mcp__cay__list_windows".into(),
            input: json!({})
        }));
        // Cay asked about its own tool itself: not a step that ran unasked, even in Ask mode.
        assert!(!out.iter().any(|o| matches!(o, Out::Unasked { .. })));
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": SID,
            "update": { "sessionUpdate": "tool_call", "toolCallId": "r", "kind": "execute", "status": "completed",
                        "rawInput": { "reviewId": "x", "threadId": "t", "action": { "type": "mcpToolCall", "toolName": "list_windows" } } } } }),
        );
        assert!(events(&out).contains(&StreamEvent::Tool {
            id: "r".into(),
            name: "Review".into(),
            input: json!({ "description": "list_windows" })
        }));
        assert!(!out.iter().any(|o| matches!(o, Out::Unasked { .. })));
    }

    #[test]
    fn a_turn_the_agent_cancels_after_a_decline_ends_normally() {
        let (mut c, _) = open(setup());
        let p = written(&c.prompt("x"))[0].clone();
        let out = feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": p["id"], "result": { "stopReason": "cancelled" } }),
        );
        assert!(matches!(&out[0], Out::Notice(n) if n.contains("declined")));
        assert!(
            matches!(&events(&out)[0], StreamEvent::Result(r) if r.subtype == "success" && !r.is_error)
        );
    }

    #[test]
    fn ending_cancels_open_requests() {
        let (mut c, _) = open(setup());
        feed(
            &mut c,
            json!({ "jsonrpc": "2.0", "id": 4, "method": "session/request_permission", "params": {
            "sessionId": SID, "toolCall": { "toolCallId": "c" }, "options": [] } }),
        );
        let lines = c.cancel_all();
        let reply: Value = serde_json::from_slice(&lines[0]).unwrap();
        assert_eq!(reply["result"]["outcome"]["outcome"], json!("cancelled"));
        assert!(c.cancel_all().is_empty());
    }
}

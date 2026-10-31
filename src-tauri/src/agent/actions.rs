//! TopBar's desktop actions as MCP sees them (Phase 2.7d, CASESTUDY D27): definitions for `tools/list`, their
//! arguments (with `ui_tree` refs and screenshot points resolved against what the run has seen), replies, and
//! audit fields that never carry the typed text. The acting itself is `crate::desktop::act`. Pure; unit-tested.

use std::time::Instant;

use serde_json::{json, Value};

use super::{mcp, tools};
use crate::desktop::plan::{self, ActError, Action, Button, ClickAt, Element, LaunchTarget, Verb};
use crate::desktop::refs::Refs;
use crate::desktop::tree::control_type_name;
use crate::desktop::Acted;

pub const FOCUS_WINDOW: &str = "focus_window";
pub const INVOKE: &str = "invoke";
pub const SET_VALUE: &str = "set_value";
pub const CLICK: &str = "click";
pub const TYPE_TEXT: &str = "type_text";
pub const PRESS_KEYS: &str = "press_keys";
pub const LAUNCH: &str = "launch";
pub const NAMES: &[&str] = &[
    FOCUS_WINDOW,
    INVOKE,
    SET_VALUE,
    CLICK,
    TYPE_TEXT,
    PRESS_KEYS,
    LAUNCH,
];

/// `mcp__topbar__click` → `click`, for TopBar's actions only.
pub fn short_name(tool: &str) -> Option<&'static str> {
    let name = tool.strip_prefix("mcp__topbar__")?;
    NAMES.iter().copied().find(|n| *n == name)
}

const APPROVED: &str =
    "The user approves each action in TopBar (never for the whole run), the screen \
    edges glow while it runs, and any mouse or keyboard input of theirs stops it.";

pub fn definitions() -> Vec<Value> {
    let hwnd = json!({ "type": "integer", "description": "A window handle from list_windows." });
    let element = json!({ "type": "integer",
        "description": "The [n] ref of an element in your latest ui_tree of this window." });
    vec![
        json!({
            "name": FOCUS_WINDOW,
            "description": format!("Brings a window to the front (restores it if minimized). {APPROVED}"),
            "inputSchema": { "type": "object", "properties": { "hwnd": hwnd }, "required": ["hwnd"] },
        }),
        json!({
            "name": INVOKE,
            "description": format!("Uses a control through UI Automation, without moving the mouse: \
                presses a button or link, toggles a checkbox, selects a list item or tab, expands or \
                collapses a menu or tree item, scrolls it into view, or focuses it. Name the element by its \
                [n] ref from ui_tree (call ui_tree again if the window changed). The window doesn't need to \
                be in front. Prefer this over click. {APPROVED}"),
            "inputSchema": { "type": "object", "properties": {
                "hwnd": hwnd, "element": element,
                "action": { "type": "string",
                    "enum": ["invoke", "toggle", "select", "expand", "collapse", "scrollIntoView", "focus"],
                    "description": "What to do; leave out for the element's main action." },
            }, "required": ["hwnd", "element"] },
        }),
        json!({
            "name": SET_VALUE,
            "description": format!("Replaces the whole text of a field (edit, combo box) through UI \
                Automation, by its [n] ref from ui_tree. Never for password fields. The reply says whether \
                the field now holds the value; if not, focus it and use type_text. At most 1000 characters. \
                {APPROVED}"),
            "inputSchema": { "type": "object", "properties": {
                "hwnd": hwnd, "element": element, "value": { "type": "string" },
            }, "required": ["hwnd", "element", "value"] },
        }),
        json!({
            "name": CLICK,
            "description": format!("Clicks with the real mouse (brings the window to the front first). \
                Give an element's [n] ref from ui_tree (preferred), or x, y in physical screen px (as in \
                ui_tree bounds), or x, y in image px of your latest screenshot of this window with \
                from: \"screenshot\". Only when invoke can't do it. {APPROVED}"),
            "inputSchema": { "type": "object", "properties": {
                "hwnd": hwnd, "element": element,
                "x": { "type": "integer" }, "y": { "type": "integer" },
                "from": { "type": "string", "enum": ["screen", "screenshot"],
                    "description": "What x, y are measured in (default screen)." },
                "button": { "type": "string", "enum": ["left", "right", "double"] },
            }, "required": ["hwnd"] },
        }),
        json!({
            "name": TYPE_TEXT,
            "description": format!("Types text with the real keyboard into the focused field of a window \
                (brings it to the front first; focus the field with invoke action \"focus\" or click). A \
                newline presses Enter, a tab presses Tab. At most 500 characters, typed at about 10 a second. Never passwords, tokens or \
                other secrets (refused). Prefer set_value. {APPROVED}"),
            "inputSchema": { "type": "object", "properties": {
                "hwnd": hwnd, "text": { "type": "string" },
            }, "required": ["hwnd", "text"] },
        }),
        json!({
            "name": PRESS_KEYS,
            "description": format!("Presses key chords in a window, in order (brings it to the front \
                first), e.g. [\"Ctrl+S\"] or [\"Ctrl+A\", \"Delete\"]. At most 5. Modifiers: Ctrl, Alt, Shift; \
                keys: letters, digits, F1–F24, Enter, Tab, Esc, Space, Backspace, Delete, Insert, Home, End, \
                PageUp, PageDown, arrows, Menu, Plus, Minus, Comma, Period. Win-key chords, Alt+Tab, \
                Ctrl+Esc and Ctrl+Shift+Esc are refused. {APPROVED}"),
            "inputSchema": { "type": "object", "properties": {
                "hwnd": hwnd,
                "keys": { "type": "array", "items": { "type": "string" } },
            }, "required": ["hwnd", "keys"] },
        }),
        json!({
            "name": LAUNCH,
            "description": format!("Opens an https/http link in the default browser, a Settings page \
                (ms-settings:…), or a Start-menu app by its name (no paths, no arguments: run programs and \
                scripts with the shell instead). {APPROVED}"),
            "inputSchema": { "type": "object", "properties": {
                "url": { "type": "string" }, "app": { "type": "string" },
            } },
        }),
    ]
}

/// A call's arguments, checked but not yet resolved against what the run has seen.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub hwnd: Option<i64>,
    kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Focus,
    Invoke {
        element: usize,
        verb: Verb,
    },
    SetValue {
        element: usize,
        value: String,
    },
    ClickElement {
        element: usize,
        button: Button,
    },
    ClickPoint {
        x: i32,
        y: i32,
        screenshot: bool,
        button: Button,
    },
    Type {
        text: String,
    },
    Keys {
        chords: Vec<plan::Chord>,
    },
    Launch(LaunchTarget),
}

fn int(args: &Value, key: &str) -> Option<i64> {
    let v = args.get(key).filter(|v| !v.is_null())?;
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

pub fn parse(name: &str, args: &Value) -> Result<Call, String> {
    let hwnd = int(args, "hwnd");
    let need_hwnd = || hwnd.ok_or_else(|| format!("{name} needs an hwnd from list_windows"));
    let element = || {
        int(args, "element")
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| format!("{name} needs an element: an [n] ref from ui_tree"))
    };
    let button = match text(args, "button").unwrap_or("left") {
        "left" | "" => Button::Left,
        "right" => Button::Right,
        "double" => Button::Double,
        other => {
            return Err(format!(
                "button must be left, right or double, not {other:?}"
            ))
        }
    };
    let kind = match name {
        FOCUS_WINDOW => {
            need_hwnd()?;
            Kind::Focus
        }
        INVOKE => {
            need_hwnd()?;
            Kind::Invoke {
                element: element()?,
                verb: Verb::parse(text(args, "action"))?,
            }
        }
        SET_VALUE => {
            need_hwnd()?;
            let value = text(args, "value").ok_or("set_value needs a value (text)")?;
            Kind::SetValue {
                element: element()?,
                value: value.to_string(),
            }
        }
        CLICK => {
            need_hwnd()?;
            if args.get("element").is_some_and(|v| !v.is_null()) {
                Kind::ClickElement {
                    element: element()?,
                    button,
                }
            } else {
                let (Some(x), Some(y)) = (int(args, "x"), int(args, "y")) else {
                    return Err("click needs an element ref, or x and y".into());
                };
                let screenshot = match text(args, "from").unwrap_or("screen") {
                    "screen" | "" => false,
                    "screenshot" => true,
                    other => {
                        return Err(format!("from must be screen or screenshot, not {other:?}"))
                    }
                };
                let clamp = |v: i64| v.clamp(-100_000, 100_000) as i32;
                Kind::ClickPoint {
                    x: clamp(x),
                    y: clamp(y),
                    screenshot,
                    button,
                }
            }
        }
        TYPE_TEXT => {
            need_hwnd()?;
            Kind::Type {
                text: text(args, "text")
                    .ok_or("type_text needs text")?
                    .to_string(),
            }
        }
        PRESS_KEYS => {
            need_hwnd()?;
            let keys: Vec<&str> = match args.get("keys") {
                Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
                Some(Value::String(s)) => vec![s.as_str()],
                _ => return Err("press_keys needs keys, e.g. [\"Ctrl+S\"]".into()),
            };
            if keys.is_empty() || keys.len() > plan::MAX_CHORDS {
                return Err(format!("press_keys takes 1 to {} chords", plan::MAX_CHORDS));
            }
            let chords = keys
                .iter()
                .map(|k| plan::parse_chord(k))
                .collect::<Result<Vec<_>, _>>()?;
            Kind::Keys { chords }
        }
        LAUNCH => match (text(args, "url"), text(args, "app")) {
            (Some(url), None) => Kind::Launch(LaunchTarget::Url(
                plan::launch_url(url).map_err(|e| e.message())?,
            )),
            (None, Some(app)) if !app.trim().is_empty() => {
                Kind::Launch(LaunchTarget::App(app.trim().to_string()))
            }
            _ => return Err("launch needs either url or app".into()),
        },
        _ => return Err("unknown tool".into()),
    };
    Ok(Call { hwnd, kind })
}

/// The action, with refs and screenshot points looked up in what `run` has seen of the window. Also the
/// window's process at that look (so a reused HWND is refused).
pub fn resolve(
    call: &Call,
    run: &str,
    refs: &Refs,
    now: Instant,
) -> Result<(Action, Option<u32>), ActError> {
    let hwnd = call.hwnd.unwrap_or(0);
    let element = |n: usize| refs.element(run, hwnd, n, now);
    Ok(match &call.kind {
        Kind::Focus => (Action::Focus, None),
        Kind::Invoke { element: n, verb } => {
            let (element, pid) = element(*n)?;
            (
                Action::Invoke {
                    element,
                    verb: *verb,
                },
                Some(pid),
            )
        }
        Kind::SetValue { element: n, value } => {
            plan::check_text(value, true)?;
            let (element, pid) = element(*n)?;
            (
                Action::SetValue {
                    element,
                    value: value.clone(),
                },
                Some(pid),
            )
        }
        Kind::ClickElement { element: n, button } => {
            let (element, pid) = element(*n)?;
            (
                Action::Click {
                    at: ClickAt::Element(element),
                    button: *button,
                },
                Some(pid),
            )
        }
        Kind::ClickPoint {
            x,
            y,
            screenshot: false,
            button,
        } => (
            Action::Click {
                at: ClickAt::Point { x: *x, y: *y },
                button: *button,
            },
            None,
        ),
        Kind::ClickPoint {
            x,
            y,
            screenshot: true,
            button,
        } => {
            let (view, pid) = refs.view(run, hwnd, now)?;
            (
                Action::Click {
                    at: ClickAt::Screenshot { x: *x, y: *y, view },
                    button: *button,
                },
                Some(pid),
            )
        }
        Kind::Type { text } => {
            plan::type_units(text)?;
            (Action::TypeText { text: text.clone() }, None)
        }
        Kind::Keys { chords } => (
            Action::PressKeys {
                chords: chords.clone(),
            },
            None,
        ),
        Kind::Launch(target) => (Action::Launch(target.clone()), None),
    })
}

/// How an approval names an element: `"Save"`, or `the edit field` when it has no name.
pub fn element_label(e: &Element) -> String {
    let name = e.name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        format!("the {} element", control_type_name(e.control_type))
    } else {
        format!("\"{}\"", super::describe::truncate(&name, 48))
    }
}

/// The element a call names (for its approval), if its ref is current.
pub fn element_for(
    tool: &str,
    input: &Value,
    run: &str,
    refs: &Refs,
    now: Instant,
) -> Option<String> {
    let name = short_name(tool)?;
    let call = parse(name, input).ok()?;
    let n = match call.kind {
        Kind::Invoke { element, .. }
        | Kind::SetValue { element, .. }
        | Kind::ClickElement { element, .. } => element,
        _ => return None,
    };
    refs.element(run, call.hwnd?, n, now)
        .ok()
        .map(|(e, _)| element_label(&e))
}

/// The full text to type or set, for the command window's approval card.
pub fn detail(tool: &str, input: &Value) -> Option<String> {
    match short_name(tool)? {
        TYPE_TEXT => text(input, "text").map(str::to_string),
        SET_VALUE => text(input, "value").map(str::to_string),
        _ => None,
    }
}

/// The input as the audit keeps it: typed and set text replaced by its size.
pub fn redact(tool: &str, input: &Value) -> Value {
    let mut out = input.clone();
    let key = match short_name(tool) {
        Some(TYPE_TEXT) => "text",
        Some(SET_VALUE) => "value",
        _ => return out,
    };
    if let Some(s) = input.get(key).and_then(Value::as_str) {
        out[key] = json!({ "chars": s.chars().count(), "lines": s.lines().count().max(1) });
    }
    out
}

/// The `tools/call` result for an action: what was done and what's in front now, or the error.
pub fn reply(rpc_id: Value, result: &Result<Acted, ActError>) -> Value {
    let acted = match result {
        Err(e) => return tools::refused(rpc_id, &format!("{}: {}", e.code(), e.message())),
        Ok(a) => a,
    };
    let mut text = acted.what.clone();
    if let Some(t) = &acted.target {
        text.push_str(&format!(" in {}", tools::target_label(t)));
    }
    text.push('.');
    if let Some(note) = &acted.note {
        text.push_str(&format!(" {note}"));
    }
    let front =
        acted
            .foreground
            .as_ref()
            .map_or("the desktop (or a TopBar window)".to_string(), |w| {
                let app = w.app.as_deref().unwrap_or("");
                format!("{app} · {} (hwnd {})", w.title, w.hwnd)
            });
    text.push_str(&format!(
        " In front now: {front}. Look again before the next action."
    ));
    mcp::result(
        rpc_id,
        json!({ "content": [{ "type": "text", "text": text }] }),
    )
}

/// The audit line for an action: tool, target, what happened; text only as its size.
pub fn audit_fields(tool: &str, input: &Value, result: &Result<Acted, ActError>) -> Value {
    let input = redact(&format!("mcp__topbar__{tool}"), input);
    match result {
        Err(e) => json!({ "tool": tool, "input": input, "ok": false, "error": e.code() }),
        Ok(a) => json!({ "tool": tool, "input": input, "ok": true, "done": a.what, "note": a.note,
            "target": a.target.as_ref().map(|t| json!({ "hwnd": t.hwnd, "app": t.app, "title": t.title })) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::logic::Rect;
    use crate::desktop::plan::View;
    use crate::desktop::Target;

    #[test]
    fn names_and_definitions() {
        assert_eq!(short_name("mcp__topbar__click"), Some("click"));
        assert_eq!(short_name("mcp__topbar__screenshot"), None);
        let defs = definitions();
        let names: Vec<_> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert_eq!(names, NAMES);
        assert!(
            defs.iter().all(|d| d.get("annotations").is_none()),
            "no hints that skip asking"
        );
        assert!(defs.iter().all(|d| d["description"]
            .as_str()
            .unwrap()
            .contains("never for the whole run")));
    }

    #[test]
    fn parsing() {
        assert!(parse(FOCUS_WINDOW, &json!({})).is_err());
        assert!(parse(FOCUS_WINDOW, &json!({ "hwnd": "12" })).is_ok());
        assert!(parse(INVOKE, &json!({ "hwnd": 1 })).is_err());
        assert!(parse(
            INVOKE,
            &json!({ "hwnd": 1, "element": 3, "action": "smash" })
        )
        .is_err());
        assert!(parse(CLICK, &json!({ "hwnd": 1 })).is_err());
        assert!(parse(
            CLICK,
            &json!({ "hwnd": 1, "x": 5, "y": 6, "button": "middle" })
        )
        .is_err());
        assert!(parse(PRESS_KEYS, &json!({ "hwnd": 1, "keys": ["Win+R"] }))
            .unwrap_err()
            .contains("Windows itself"));
        assert!(parse(
            PRESS_KEYS,
            &json!({ "hwnd": 1, "keys": ["a", "b", "c", "d", "e", "f"] })
        )
        .is_err());
        assert!(parse(PRESS_KEYS, &json!({ "hwnd": 1, "keys": "Ctrl+S" })).is_ok());
        assert!(parse(LAUNCH, &json!({ "url": "file:///C:/x.exe" })).is_err());
        assert!(parse(LAUNCH, &json!({ "url": "https://a.b", "app": "x" })).is_err());
        assert!(parse(LAUNCH, &json!({ "app": "Notepad" })).is_ok());
    }

    #[test]
    fn resolving_refs_and_views() {
        let now = Instant::now();
        let mut refs = Refs::default();
        let save = Element {
            runtime_id: vec![1],
            control_type: 50000,
            name: "Save".into(),
            ..Element::default()
        };
        refs.put_tree("r", 7, 99, vec![Element::default(), save.clone()], now);

        let call = parse(INVOKE, &json!({ "hwnd": 7, "element": 1 })).unwrap();
        let (action, pid) = resolve(&call, "r", &refs, now).unwrap();
        assert_eq!(
            action,
            Action::Invoke {
                element: save.clone(),
                verb: Verb::Default
            }
        );
        assert_eq!(pid, Some(99));
        assert_eq!(
            element_for(
                "mcp__topbar__invoke",
                &json!({ "hwnd": 7, "element": 1 }),
                "r",
                &refs,
                now
            ),
            Some("\"Save\"".into())
        );
        let stale = parse(INVOKE, &json!({ "hwnd": 7, "element": 5 })).unwrap();
        assert_eq!(
            resolve(&stale, "r", &refs, now).unwrap_err(),
            ActError::ElementGone
        );
        assert_eq!(
            resolve(&call, "other", &refs, now).unwrap_err(),
            ActError::ElementGone
        );

        let shot = parse(
            CLICK,
            &json!({ "hwnd": 7, "x": 10, "y": 20, "from": "screenshot" }),
        )
        .unwrap();
        assert_eq!(
            resolve(&shot, "r", &refs, now).unwrap_err(),
            ActError::StaleView
        );
        let view = View {
            origin: Rect::from_ltrb(100, 100, 300, 300),
            scale: 2.0,
            frame: Rect::from_ltrb(100, 100, 300, 300),
        };
        refs.put_view("r", 7, 99, view, now);
        let (
            Action::Click {
                at: ClickAt::Screenshot { view: v, .. },
                ..
            },
            _,
        ) = resolve(&shot, "r", &refs, now).unwrap()
        else {
            panic!()
        };
        assert_eq!(v.screen_point(10, 20), (120, 140));

        let secret = parse(
            TYPE_TEXT,
            &json!({ "hwnd": 7, "text": "ghp_0123456789abcdefghijABCDEFGHIJ" }),
        )
        .unwrap();
        assert!(matches!(
            resolve(&secret, "r", &refs, now),
            Err(ActError::Refused(_))
        ));
        assert_eq!(
            element_label(&Element {
                control_type: 50004,
                ..Element::default()
            }),
            "the edit element"
        );
    }

    #[test]
    fn text_stays_out_of_the_audit() {
        let input = json!({ "hwnd": 1, "text": "Dear team,\nhello" });
        let r = redact("mcp__topbar__type_text", &input);
        assert_eq!(r["text"], json!({ "chars": 16, "lines": 2 }));
        assert_eq!(
            detail("mcp__topbar__type_text", &input).as_deref(),
            Some("Dear team,\nhello")
        );
        let line = audit_fields(TYPE_TEXT, &input, &Err(ActError::PasswordField));
        assert!(!line.to_string().contains("Dear team"));
        assert_eq!(line["error"], "passwordField");
        assert_eq!(
            redact("mcp__topbar__click", &json!({ "x": 1 })),
            json!({ "x": 1 })
        );
    }

    #[test]
    fn replies() {
        let ok = Ok(Acted {
            what: "Invoked button \"Save\"".into(),
            note: None,
            target: Some(Target {
                hwnd: 5,
                pid: 1,
                app: Some("notepad".into()),
                title: "notes.txt".into(),
            }),
            foreground: None,
        });
        let r = reply(json!(3), &ok);
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.starts_with("Invoked button \"Save\" in Notepad · notes.txt."),
            "{text}"
        );
        assert!(r["result"].get("isError").is_none());
        let r = reply(json!(4), &Err(ActError::Aborted { done: 1, total: 3 }));
        assert_eq!(r["result"]["isError"], true);
        assert!(r.to_string().contains("aborted: "));
    }
}

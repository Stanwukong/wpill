//! Cay's desktop tools as MCP sees them (Phase 2.7c): their definitions for `tools/list`, their arguments, and
//! their replies (text, or an image block plus text). The looking itself is `crate::desktop`. Pure; unit-tested.

use base64::Engine;
use serde_json::{json, Value};

use super::mcp;
use crate::desktop::logic::max_edge;
use crate::desktop::tree::{Caps, Cut, Filter};
use crate::desktop::{Look, LookError, Outcome, Rect, Target};

pub const LIST_WINDOWS: &str = "list_windows";
pub const FOCUSED_WINDOW: &str = "focused_window";
pub const SCREENSHOT: &str = "screenshot";
pub const UI_TREE: &str = "ui_tree";
pub const NAMES: &[&str] = &[LIST_WINDOWS, FOCUSED_WINDOW, SCREENSHOT, UI_TREE];

/// `mcp__cay__screenshot` → `screenshot`, for Cay's desktop tools only.
pub fn short_name(tool: &str) -> Option<&'static str> {
    let name = tool.strip_prefix("mcp__cay__")?;
    NAMES.iter().copied().find(|n| *n == name)
}

pub fn definitions() -> Vec<Value> {
    let hwnd = json!({ "type": "integer", "description": "A window handle from list_windows." });
    vec![
        json!({
            "name": LIST_WINDOWS,
            "description": "Lists the user's open windows (like Alt+Tab), front to back: hwnd, app, title, \
                bounds in physical screen pixels, minimized, focused. Read-only. The user approves this in \
                Cay and sees the screen edges glow.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": FOCUSED_WINDOW,
            "description": "The window in front (the one the user is working in), in the same shape as \
                list_windows, or null. Read-only; approved by the user like list_windows.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": SCREENSHOT,
            "description": "A screenshot of one window (hwnd from list_windows; works when it's covered) or, \
                without hwnd, of the primary screen. At most 1568 px on the long edge. The reply says where \
                the image is on screen (origin, scale = screen px per image px). Costs image tokens (about \
                width x height / 750): try ui_tree first for text and controls, and keep it small with region \
                (just the part you need) and maxEdge. Protected video can come out black. Read-only; the user \
                approves it and sees the screen edges glow.",
            "inputSchema": { "type": "object", "properties": {
                "hwnd": hwnd,
                "region": { "type": "object", "description": "Only this part, in physical screen px (as \
                    list_windows bounds and ui_tree's bounds: true give them), cut to the window or screen.",
                    "properties": {
                        "x": { "type": "integer" }, "y": { "type": "integer" },
                        "width": { "type": "integer" }, "height": { "type": "integer" },
                    }, "required": ["x", "y", "width", "height"] },
                "maxEdge": { "type": "integer", "description": "Longest image edge in px (default and at \
                    most 1568, at least 200). 800 is often enough to read a window." },
            } },
        }),
        json!({
            "name": UI_TREE,
            "description": "A window's UI Automation tree as an indented outline: control type, \"name\", \
                = \"value\", #automationId and flags (focused, disabled, password), each line starting with \
                its [n] ref, which invoke, set_value and click take as `element` (refs last until the next \
                ui_tree of that window). Unnamed layout containers are left out (their contents move up a \
                level, so ref numbers skip), and so is what's offscreen. More exact than a screenshot for \
                text, buttons and fields; on big windows (browsers, Office) keep it small with maxDepth. \
                Password fields never show a value. Read-only; the user approves it and sees the screen \
                edges glow.",
            "inputSchema": { "type": "object", "properties": {
                "hwnd": hwnd,
                "maxDepth": { "type": "integer", "description": "Levels to walk (default 12, at most 25)." },
                "maxNodes": { "type": "integer", "description": "Elements to walk (default 250, at most 1000)." },
                "bounds": { "type": "boolean", "description": "Also each element's @x,y width x height in \
                    physical screen px and the focusable flag; only to click by coordinates (default false)." },
                "offscreen": { "type": "boolean", "description": "Also elements scrolled out of view or on \
                    hidden pages (default false)." },
                "find": { "type": "string", "description": "Only elements whose name, value or automation id \
                    contains this text (any case), each under its ancestors. The walk's limits still apply." },
                "types": { "type": "array", "items": { "type": "string" }, "description": "Only these \
                    control types, as the outline names them (button, edit, hyperlink, listitem, menuitem …)." },
            }, "required": ["hwnd"] },
        }),
    ]
}

/// A `tools/call` for one of the four: what to look at, or why the arguments are wrong.
pub fn parse(name: &str, args: &Value) -> Result<Look, String> {
    let hwnd = args.get("hwnd").filter(|v| !v.is_null());
    let hwnd = match hwnd {
        None => None,
        Some(v) => Some(
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                .ok_or("hwnd must be a number from list_windows")?,
        ),
    };
    match name {
        LIST_WINDOWS => Ok(Look::ListWindows),
        FOCUSED_WINDOW => Ok(Look::FocusedWindow),
        SCREENSHOT => Ok(Look::Screenshot {
            hwnd,
            region: region(args.get("region"))?,
            max_edge: max_edge(args.get("maxEdge").and_then(Value::as_u64)),
        }),
        UI_TREE => Ok(Look::UiTree {
            hwnd: hwnd.ok_or("ui_tree needs an hwnd from list_windows")?,
            filter: filter(args)?,
            caps: Caps {
                bounds: args.get("bounds").and_then(Value::as_bool) == Some(true),
                offscreen: args.get("offscreen").and_then(Value::as_bool) == Some(true),
                ..Caps::new(
                    args.get("maxDepth").and_then(Value::as_u64),
                    args.get("maxNodes").and_then(Value::as_u64),
                )
            },
        }),
        _ => Err("unknown tool".into()),
    }
}

/// A tree's optional `find` text and `types` list.
fn filter(args: &Value) -> Result<Filter, String> {
    let find = match args.get("find").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => Some(v.as_str().ok_or("find must be text")?),
    };
    let types: Vec<&str> = match args.get("types").filter(|v| !v.is_null()) {
        None => Vec::new(),
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Array(list)) => list
            .iter()
            .map(|t| t.as_str().ok_or("types must be control type names"))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("types must be a list of control type names".into()),
    };
    Filter::new(find, &types)
}

/// A screenshot's optional `region` (`{x, y, width, height}`, screen px).
fn region(v: Option<&Value>) -> Result<Option<Rect>, String> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    const WRONG: &str =
        "region must be {x, y, width, height} in screen px, width and height above 0";
    let int = |k: &str| {
        v.get(k)
            .and_then(Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
            .ok_or(WRONG)
    };
    let (x, y, width, height) = (int("x")?, int("y")?, int("width")?, int("height")?);
    if width <= 0 || height <= 0 {
        return Err(WRONG.into());
    }
    Ok(Some(Rect {
        x,
        y,
        width,
        height,
    }))
}

/// "Chrome · GitHub – Pull requests" (the app, then the title).
pub fn target_label(t: &Target) -> String {
    let app = t.app.as_deref().map(capitalize).unwrap_or_default();
    match (app.is_empty(), t.title.trim().is_empty()) {
        (false, false) => format!("{app} · {}", t.title.trim()),
        (false, true) => app,
        (true, false) => t.title.trim().to_string(),
        (true, true) => "a window".into(),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// The `tools/call` result for a look.
pub fn reply(rpc_id: Value, result: &Result<Outcome, LookError>) -> Value {
    let content = match result {
        Err(e) => {
            return mcp::result(
                rpc_id,
                json!({ "content": [text(&format!("{}: {}", e.code(), e.message()))], "isError": true }),
            )
        }
        Ok(Outcome::Windows(list)) => vec![text(&json!({ "windows": list }).to_string())],
        Ok(Outcome::Focused { window, cay }) => {
            let mut v = json!({ "window": window });
            if *cay {
                v["note"] = json!("Cay's own window is in front; it can't be described.");
            }
            vec![text(&v.to_string())]
        }
        Ok(Outcome::Image { image, target }) => {
            let source = target
                .as_ref()
                .map_or("the primary screen".to_string(), target_label);
            let info = json!({
                "source": source,
                "width": image.width,
                "height": image.height,
                "origin": { "x": image.origin.x, "y": image.origin.y },
                "scale": (image.scale * 1000.0).round() / 1000.0,
                "format": image.mime,
            });
            vec![
                json!({
                    "type": "image",
                    "data": base64::engine::general_purpose::STANDARD.encode(&image.bytes),
                    "mimeType": image.mime,
                }),
                text(&format!(
                    "Screenshot of {source} · {}x{} px. {info}",
                    image.width, image.height
                )),
            ]
        }
        Ok(Outcome::Tree {
            outline, target, ..
        }) => vec![text(&format!("UI of {}:\n{outline}", target_label(target)))],
    };
    mcp::result(rpc_id, json!({ "content": content }))
}

/// A look that didn't run (denied, timed out, bad arguments), as the tool's error result.
pub fn refused(rpc_id: Value, message: &str) -> Value {
    mcp::result(
        rpc_id,
        json!({ "content": [text(message)], "isError": true }),
    )
}

/// The audit line for a look: what was looked at and how it went, never the image or the tree.
pub fn audit_fields(tool: &str, result: &Result<Outcome, LookError>) -> Value {
    let target = |t: &Target| json!({ "hwnd": t.hwnd, "app": t.app, "title": t.title });
    match result {
        Err(e) => json!({ "tool": tool, "ok": false, "error": e.code() }),
        Ok(Outcome::Windows(list)) => json!({ "tool": tool, "ok": true, "windows": list.len() }),
        Ok(Outcome::Focused { window, cay }) => json!({ "tool": tool, "ok": true,
            "target": window.as_ref().map(|w| json!({ "hwnd": w.hwnd, "app": w.app, "title": w.title })),
            "cay": cay }),
        Ok(Outcome::Image { image, target: t }) => json!({ "tool": tool, "ok": true,
            "target": t.as_ref().map(target).unwrap_or(json!("screen")),
            "width": image.width, "height": image.height, "bytes": image.bytes.len(),
            "format": image.mime, "imageTokens": image_tokens(image.width, image.height) }),
        Ok(Outcome::Tree {
            nodes,
            cut,
            target: t,
            outline,
            ..
        }) => json!({ "tool": tool, "ok": true, "target": target(t), "nodes": nodes,
            "chars": outline.chars().count(),
            "cut": cut.map(|c| match c { Cut::Nodes => "nodes", Cut::Time => "time" }) }),
    }
}

/// Roughly what an image costs Claude (Anthropic's estimate: width × height / 750), for the audit.
pub fn image_tokens(width: u32, height: u32) -> u64 {
    u64::from(width) * u64::from(height) / 750
}

/// The characters of a reply's text blocks (what Claude reads besides images), for the audit.
pub fn text_chars(reply: &Value) -> usize {
    reply["result"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .map(|t| t.chars().count())
        .sum()
}

/// Adds how long a look or action took and how much text its reply holds to its audit line (D58).
pub fn with_cost(mut fields: Value, took: std::time::Duration, reply: &Value) -> Value {
    fields["ms"] = json!(took.as_millis() as u64);
    fields["replyChars"] = json!(text_chars(reply));
    fields
}

fn text(s: &str) -> Value {
    json!({ "type": "text", "text": s })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::{Image, Rect, WindowEntry};

    fn chrome() -> Target {
        Target {
            hwnd: 42,
            pid: 7,
            app: Some("chrome".into()),
            title: "GitHub".into(),
        }
    }

    #[test]
    fn names_and_definitions() {
        assert_eq!(short_name("mcp__cay__screenshot"), Some("screenshot"));
        assert_eq!(short_name("mcp__cay__approve"), None);
        assert_eq!(short_name("mcp__other__screenshot"), None);
        let defs = definitions();
        let names: Vec<_> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert_eq!(names, NAMES);
        // No readOnlyHint: nothing may treat these as "no need to ask".
        assert!(defs.iter().all(|d| d.get("annotations").is_none()));
    }

    #[test]
    fn arguments() {
        assert_eq!(parse(LIST_WINDOWS, &json!({})), Ok(Look::ListWindows));
        assert_eq!(
            parse(SCREENSHOT, &json!({})),
            Ok(Look::Screenshot {
                hwnd: None,
                region: None,
                max_edge: 1568
            })
        );
        assert_eq!(
            parse(SCREENSHOT, &json!({ "hwnd": "1234", "maxEdge": 800 })),
            Ok(Look::Screenshot {
                hwnd: Some(1234),
                region: None,
                max_edge: 800
            })
        );
        assert_eq!(
            parse(
                SCREENSHOT,
                &json!({ "hwnd": 5, "region": { "x": -10, "y": 20, "width": 300, "height": 40 } })
            ),
            Ok(Look::Screenshot {
                hwnd: Some(5),
                region: Some(Rect {
                    x: -10,
                    y: 20,
                    width: 300,
                    height: 40
                }),
                max_edge: 1568
            })
        );
        for bad in [
            json!({ "x": 0, "y": 0, "width": 0, "height": 10 }),
            json!({ "x": 0, "y": 0, "width": 10 }),
            json!({ "x": "a", "y": 0, "width": 10, "height": 10 }),
            json!([0, 0, 10, 10]),
        ] {
            assert!(parse(SCREENSHOT, &json!({ "region": bad })).is_err());
        }
        assert!(parse(UI_TREE, &json!({})).is_err());
        assert!(parse(SCREENSHOT, &json!({ "hwnd": "abc" })).is_err());
        let Ok(Look::UiTree { caps, .. }) =
            parse(UI_TREE, &json!({ "hwnd": 5, "maxNodes": 50000 }))
        else {
            panic!()
        };
        assert_eq!(caps.nodes, crate::desktop::tree::MAX_NODES);
        assert!(!caps.bounds && !caps.offscreen);
        let Ok(Look::UiTree { caps, .. }) = parse(
            UI_TREE,
            &json!({ "hwnd": 5, "bounds": true, "offscreen": true }),
        ) else {
            panic!()
        };
        assert!(caps.bounds && caps.offscreen);
        let Ok(Look::UiTree { filter, .. }) = parse(
            UI_TREE,
            &json!({ "hwnd": 5, "find": "Save", "types": ["Button", "edit"] }),
        ) else {
            panic!()
        };
        assert_eq!(filter.find.as_deref(), Some("save"));
        assert_eq!(filter.types, ["button", "edit"]);
        let Ok(Look::UiTree { filter, .. }) =
            parse(UI_TREE, &json!({ "hwnd": 5, "types": "edit" }))
        else {
            panic!()
        };
        assert_eq!(filter.types, ["edit"]);
        assert!(parse(UI_TREE, &json!({ "hwnd": 5, "types": ["nope"] })).is_err());
        assert!(parse(UI_TREE, &json!({ "hwnd": 5, "find": 3 })).is_err());
        assert_eq!(caps.bytes, crate::desktop::tree::DEFAULT_BYTES);
    }

    #[test]
    fn labels() {
        assert_eq!(target_label(&chrome()), "Chrome · GitHub");
        let bare = Target {
            app: None,
            title: " ".into(),
            ..chrome()
        };
        assert_eq!(target_label(&bare), "a window");
    }

    #[test]
    fn image_reply_and_audit() {
        let result = Ok(Outcome::Image {
            image: Image {
                bytes: vec![1, 2, 3],
                mime: "image/png",
                width: 1568,
                height: 882,
                origin: Rect::from_ltrb(0, 0, 1920, 1080),
                frame: Rect::from_ltrb(0, 0, 1920, 1080),
                scale: 1920.0 / 1568.0,
            },
            target: Some(chrome()),
        });
        let r = reply(json!(9), &result);
        assert_eq!(r["id"], 9);
        let content = &r["result"]["content"];
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[0]["mimeType"], "image/png");
        assert_eq!(content[0]["data"], "AQID");
        assert!(content[1]["text"]
            .as_str()
            .unwrap()
            .starts_with("Screenshot of Chrome · GitHub · 1568x882 px."));
        assert!(r["result"].get("isError").is_none());

        let audit = audit_fields(SCREENSHOT, &result);
        assert_eq!(audit["bytes"], 3);
        assert_eq!(audit["imageTokens"], 1843);
        let costed = with_cost(audit.clone(), std::time::Duration::from_millis(42), &r);
        assert_eq!(costed["ms"], 42);
        assert_eq!(
            costed["replyChars"],
            content[1]["text"].as_str().unwrap().chars().count()
        );
        assert_eq!(audit["target"]["app"], "chrome");
        assert!(!audit.to_string().contains("AQID"), "never the image");
    }

    #[test]
    fn error_and_text_replies() {
        let r = reply(json!(1), &Err(LookError::WindowGone));
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("windowGone: "));
        let w = WindowEntry {
            hwnd: 42,
            app: Some("notepad".into()),
            title: "notes.txt".into(),
            bounds: Rect::default(),
            minimized: false,
            focused: true,
        };
        let r = reply(json!(2), &Ok(Outcome::Windows(vec![w])));
        let listed: Value =
            serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(listed["windows"][0]["hwnd"], 42);
        let r = reply(
            json!(3),
            &Ok(Outcome::Focused {
                window: None,
                cay: true,
            }),
        );
        assert!(r.to_string().contains("Cay's own window"));
        let tree = Ok(Outcome::Tree {
            outline: "window \"x\"\n".into(),
            nodes: 1,
            cut: Some(Cut::Time),
            target: chrome(),
            elements: vec![],
        });
        assert!(reply(json!(4), &tree)
            .to_string()
            .contains("UI of Chrome · GitHub"));
        assert_eq!(audit_fields(UI_TREE, &tree)["cut"], "time");
        assert!(!audit_fields(UI_TREE, &tree)
            .to_string()
            .contains("window \\\"x"));
    }
}

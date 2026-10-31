//! A window's UI Automation tree as Claude reads it: nodes in depth-first order, capped, and written as a short
//! indented outline (fewer tokens than JSON). Each line starts with its ref (`[12]`), which the action tools
//! (2.7d) take to name an element. Password fields never carry a value. Pure; unit-tested.

use super::logic::Rect;
use super::plan::Element;

pub const DEFAULT_DEPTH: u32 = 12;
pub const MAX_DEPTH: u32 = 25;
pub const DEFAULT_NODES: u32 = 400;
pub const MAX_NODES: u32 = 1000;
/// Outline text cap: well under Claude Code's 25,000-token MCP result limit.
pub const MAX_BYTES: usize = 60 * 1024;
/// Longest name or value kept per node.
const MAX_FIELD: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Caps {
    pub depth: u32,
    pub nodes: u32,
    pub bytes: usize,
}

impl Caps {
    /// From the tool's optional `maxDepth` / `maxNodes`, clamped.
    pub fn new(depth: Option<u64>, nodes: Option<u64>) -> Self {
        let clamp = |v: Option<u64>, default: u32, max: u32| {
            v.map_or(default, |v| v.clamp(1, u64::from(max)) as u32)
        };
        Self {
            depth: clamp(depth, DEFAULT_DEPTH, MAX_DEPTH),
            nodes: clamp(nodes, DEFAULT_NODES, MAX_NODES),
            bytes: MAX_BYTES,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    /// 0 for the window itself.
    pub depth: u32,
    pub control_type: i32,
    pub name: String,
    pub value: Option<String>,
    pub automation_id: String,
    pub rect: Rect,
    pub enabled: bool,
    pub focusable: bool,
    pub focused: bool,
    pub offscreen: bool,
    pub password: bool,
    /// UIA's runtime id, to find the element again for an action.
    pub runtime_id: Vec<i32>,
}

/// The nodes as action targets, in ref order.
pub fn elements(nodes: &[Node]) -> Vec<Element> {
    nodes
        .iter()
        .map(|n| Element {
            runtime_id: n.runtime_id.clone(),
            control_type: n.control_type,
            name: n.name.clone(),
            automation_id: n.automation_id.clone(),
            rect: n.rect,
        })
        .collect()
}

/// Why the walk stopped early.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cut {
    Nodes,
    Time,
}

/// The outline: one line per node, two spaces per level, then its ref, e.g.
/// `  [3] button "Save" #saveBtn @812,40 96x32 focusable`. Stops at `caps.bytes` and says so.
pub fn render(nodes: &[Node], cut: Option<Cut>, caps: &Caps) -> String {
    let mut out = String::new();
    let mut shown = 0usize;
    for (i, node) in nodes.iter().enumerate() {
        let line = line(i, node);
        if out.len() + line.len() + 1 > caps.bytes {
            break;
        }
        out.push_str(&line);
        out.push('\n');
        shown += 1;
    }
    let hidden = nodes.len() - shown;
    if hidden > 0 {
        out.push_str(&format!(
            "… {hidden} more elements not shown (text limit). Use a smaller maxDepth, or a screenshot.\n"
        ));
    }
    match cut {
        Some(Cut::Nodes) => out.push_str(&format!(
            "… stopped at {} elements (maxNodes); there are more.\n",
            caps.nodes
        )),
        Some(Cut::Time) => {
            out.push_str("… stopped early: the app was slow to answer; there may be more.\n")
        }
        None => {}
    }
    out
}

fn line(i: usize, n: &Node) -> String {
    let mut s = "  ".repeat(n.depth as usize);
    s.push_str(&format!("[{i}] "));
    s.push_str(control_type_name(n.control_type));
    if !n.name.trim().is_empty() {
        s.push_str(&format!(" {}", quoted(&n.name)));
    }
    // Belt and braces: whatever the walk collected, a password field shows no value.
    if let Some(value) = n.value.as_deref().filter(|v| !n.password && !v.is_empty()) {
        if value != n.name {
            s.push_str(&format!(" = {}", quoted(value)));
        }
    }
    if !n.automation_id.is_empty() && n.automation_id.len() <= 64 {
        s.push_str(&format!(" #{}", n.automation_id));
    }
    if !n.rect.is_empty() {
        s.push_str(&format!(
            " @{},{} {}x{}",
            n.rect.x, n.rect.y, n.rect.width, n.rect.height
        ));
    }
    for (on, flag) in [
        (!n.enabled, "disabled"),
        (n.focusable, "focusable"),
        (n.focused, "focused"),
        (n.offscreen, "offscreen"),
        (n.password, "password"),
    ] {
        if on {
            s.push(' ');
            s.push_str(flag);
        }
    }
    s
}

/// One line, cut, in double quotes with `"` and `\` escaped.
fn quoted(text: &str) -> String {
    let one: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_FIELD + 1)
        .collect();
    let one = if one.chars().count() > MAX_FIELD {
        let mut cut: String = one.chars().take(MAX_FIELD - 1).collect();
        cut.push('…');
        cut
    } else {
        one
    };
    format!("\"{}\"", one.replace('\\', "\\\\").replace('"', "\\\""))
}

/// UIA control type ids (`UIA_ButtonControlTypeId` = 50000 …) to short names.
pub fn control_type_name(id: i32) -> &'static str {
    const NAMES: [&str; 41] = [
        "button",
        "calendar",
        "checkbox",
        "combobox",
        "edit",
        "hyperlink",
        "image",
        "listitem",
        "list",
        "menu",
        "menubar",
        "menuitem",
        "progressbar",
        "radiobutton",
        "scrollbar",
        "slider",
        "spinner",
        "statusbar",
        "tab",
        "tabitem",
        "text",
        "toolbar",
        "tooltip",
        "tree",
        "treeitem",
        "custom",
        "group",
        "thumb",
        "datagrid",
        "dataitem",
        "document",
        "splitbutton",
        "window",
        "pane",
        "header",
        "headeritem",
        "table",
        "titlebar",
        "separator",
        "semanticzoom",
        "appbar",
    ];
    usize::try_from(id - 50000)
        .ok()
        .and_then(|i| NAMES.get(i))
        .copied()
        .unwrap_or("element")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(depth: u32, control_type: i32, name: &str) -> Node {
        Node {
            depth,
            control_type,
            name: name.into(),
            enabled: true,
            ..Node::default()
        }
    }

    #[test]
    fn caps_are_clamped() {
        assert_eq!(Caps::new(None, None).depth, DEFAULT_DEPTH);
        assert_eq!(Caps::new(Some(99), Some(0)).depth, MAX_DEPTH);
        assert_eq!(Caps::new(Some(99), Some(0)).nodes, 1);
        assert_eq!(Caps::new(None, Some(5000)).nodes, MAX_NODES);
    }

    #[test]
    fn outline_lines() {
        let mut save = node(1, 50000, "Save");
        save.automation_id = "saveBtn".into();
        save.rect = Rect::from_ltrb(812, 40, 908, 72);
        save.focusable = true;
        let mut edit = node(1, 50004, "Text editor");
        edit.value = Some("hello \"world\"\nline 2".into());
        edit.focused = true;
        let mut off = node(2, 50020, "x");
        off.enabled = false;
        off.offscreen = true;
        let text = render(
            &[node(0, 50032, "notes.txt - Notepad"), save, edit, off],
            None,
            &Caps::new(None, None),
        );
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines[0], "[0] window \"notes.txt - Notepad\"");
        assert_eq!(
            lines[1],
            "  [1] button \"Save\" #saveBtn @812,40 96x32 focusable"
        );
        assert_eq!(
            lines[2],
            "  [2] edit \"Text editor\" = \"hello \\\"world\\\" line 2\" focused"
        );
        assert_eq!(lines[3], "    [3] text \"x\" disabled offscreen");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn passwords_never_show_a_value() {
        let mut pw = node(1, 50004, "Password");
        pw.value = Some("hunter2".into());
        pw.password = true;
        let text = render(&[pw], None, &Caps::new(None, None));
        assert!(!text.contains("hunter2"));
        assert!(text.contains("[0] edit \"Password\" password"));
    }

    #[test]
    fn caps_and_cuts_are_reported() {
        let nodes: Vec<Node> = (0..2000)
            .map(|i| node(1, 50020, &format!("item {i} {}", "x".repeat(100))))
            .collect();
        let caps = Caps::new(None, None);
        let text = render(&nodes, Some(Cut::Nodes), &caps);
        assert!(text.len() <= caps.bytes + 200);
        assert!(text.contains("more elements not shown"));
        assert!(text.contains("stopped at 400 elements"));
        let text = render(&nodes[..2], Some(Cut::Time), &caps);
        assert!(text.contains("slow to answer"));
        let long = node(0, 50020, &"y".repeat(500));
        assert_eq!(
            line(0, &long).chars().count(),
            "[0] text \"\"".len() + MAX_FIELD
        );
    }

    #[test]
    fn control_types() {
        assert_eq!(control_type_name(50000), "button");
        assert_eq!(control_type_name(50033), "pane");
        assert_eq!(control_type_name(50040), "appbar");
        assert_eq!(control_type_name(50041), "element");
        assert_eq!(control_type_name(0), "element");
    }
}

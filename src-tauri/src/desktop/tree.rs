//! A window's UI Automation tree as Claude reads it: nodes in depth-first order, capped, and written as a short
//! indented outline (fewer tokens than JSON). Each line starts with its ref (`[12]`), which the action tools
//! (2.7d) take to name an element. Password fields never carry a value. Pure; unit-tested.

use super::logic::Rect;
use super::plan::Element;

pub const DEFAULT_DEPTH: u32 = 12;
pub const MAX_DEPTH: u32 = 25;
pub const DEFAULT_NODES: u32 = 250;
pub const MAX_NODES: u32 = 1000;
/// Outline text cap by default (about 6k tokens): most windows fit, and a bigger one says it was cut.
pub const DEFAULT_BYTES: usize = 24 * 1024;
/// Outline text cap when Claude asks for more elements: well under Claude Code's 25,000-token MCP result limit.
pub const MAX_BYTES: usize = 60 * 1024;
/// Longest name or value kept per node.
const MAX_FIELD: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Caps {
    pub depth: u32,
    pub nodes: u32,
    pub bytes: usize,
    /// Each line's screen position and size and the `focusable` flag (only needed to click by coordinates).
    pub bounds: bool,
    /// Also walk what UIA marks offscreen (scrolled out of view, hidden pages) and what's inside it.
    pub offscreen: bool,
}

impl Caps {
    /// From the tool's optional `maxDepth` / `maxNodes`, clamped. Asking for more nodes than the default also
    /// raises the text cap.
    pub fn new(depth: Option<u64>, nodes: Option<u64>) -> Self {
        let clamp = |v: Option<u64>, default: u32, max: u32| {
            v.map_or(default, |v| v.clamp(1, u64::from(max)) as u32)
        };
        let nodes = clamp(nodes, DEFAULT_NODES, MAX_NODES);
        Self {
            depth: clamp(depth, DEFAULT_DEPTH, MAX_DEPTH),
            nodes,
            bytes: if nodes > DEFAULT_NODES {
                MAX_BYTES
            } else {
                DEFAULT_BYTES
            },
            bounds: false,
            offscreen: false,
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

/// The outline: one line per node, two spaces per level, then its ref, e.g. `  [3] button "Save" #saveBtn`
/// (plus `@812,40 96x32 focusable` with `caps.bounds`). Unnamed layout containers get no line of their own (their
/// children move up a level), so refs can skip numbers. `offscreen` is how many offscreen elements the walk left
/// out. Stops at `caps.bytes` and says so.
pub fn render(nodes: &[Node], cut: Option<Cut>, offscreen: u32, caps: &Caps) -> String {
    let mut out = String::new();
    let mut hidden = 0usize;
    // The depths of the current node's shown ancestors.
    let mut shown: Vec<u32> = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        while shown.last().is_some_and(|d| *d >= node.depth) {
            shown.pop();
        }
        if merged(node) {
            continue;
        }
        if hidden > 0 {
            hidden += 1;
            continue;
        }
        let line = line(i, node, shown.len(), caps.bounds);
        if out.len() + line.len() + 1 > caps.bytes {
            hidden = 1;
            continue;
        }
        out.push_str(&line);
        out.push('\n');
        shown.push(node.depth);
    }
    if hidden > 0 {
        out.push_str(&format!(
            "… {hidden} more elements not shown (text limit). Use a smaller maxDepth, or a screenshot.\n"
        ));
    }
    if offscreen > 0 {
        out.push_str(&format!(
            "… {offscreen} offscreen elements (and what's inside them) left out; offscreen: true includes them.\n"
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

/// A node with nothing to read or use: an unnamed pane, group, custom element, text, image or separator (its
/// children are still shown). Never the window itself or what has focus.
fn merged(n: &Node) -> bool {
    const LAYOUT: [&str; 7] = [
        "pane",
        "group",
        "custom",
        "text",
        "image",
        "separator",
        "element",
    ];
    n.depth > 0
        && n.name.trim().is_empty()
        && n.value.as_deref().is_none_or(str::is_empty)
        && !n.focused
        && LAYOUT.contains(&control_type_name(n.control_type))
}

fn line(i: usize, n: &Node, level: usize, bounds: bool) -> String {
    let mut s = "  ".repeat(level);
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
    if bounds && !n.rect.is_empty() {
        s.push_str(&format!(
            " @{},{} {}x{}",
            n.rect.x, n.rect.y, n.rect.width, n.rect.height
        ));
    }
    for (on, flag) in [
        (!n.enabled, "disabled"),
        (bounds && n.focusable, "focusable"),
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
        let nodes = [node(0, 50032, "notes.txt - Notepad"), save, edit, off];
        let text = render(&nodes, None, 0, &Caps::new(None, None));
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines[0], "[0] window \"notes.txt - Notepad\"");
        assert_eq!(lines[1], "  [1] button \"Save\" #saveBtn");
        assert_eq!(
            lines[2],
            "  [2] edit \"Text editor\" = \"hello \\\"world\\\" line 2\" focused"
        );
        assert_eq!(lines[3], "    [3] text \"x\" disabled offscreen");
        assert_eq!(lines.len(), 4);

        let caps = Caps {
            bounds: true,
            ..Caps::new(None, None)
        };
        let text = render(&nodes, None, 0, &caps);
        assert_eq!(
            text.lines().nth(1),
            Some("  [1] button \"Save\" #saveBtn @812,40 96x32 focusable")
        );
    }

    #[test]
    fn unnamed_containers_are_merged() {
        let mut focused_pane = node(2, 50033, "");
        focused_pane.focused = true;
        let nodes = [
            node(0, 50032, "App"),
            node(1, 50033, ""),        // [1] unnamed pane: merged
            node(2, 50026, ""),        // [2] unnamed group: merged
            node(3, 50000, "OK"),      // [3] one level under the window
            node(2, 50000, ""),        // [4] unnamed button: kept
            focused_pane,              // [5] has focus: kept
            node(3, 50020, "Hi"),      // [6] inside the focused pane
            node(1, 50026, "Toolbar"), // [7] named group: kept
            node(2, 50006, ""),        // [8] unnamed image: merged
        ];
        let text = render(&nodes, None, 0, &Caps::new(None, None));
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "[0] window \"App\"",
                "  [3] button \"OK\"",
                "  [4] button",
                "  [5] pane focused",
                "    [6] text \"Hi\"",
                "  [7] group \"Toolbar\"",
            ]
        );
    }

    #[test]
    fn passwords_never_show_a_value() {
        let mut pw = node(1, 50004, "Password");
        pw.value = Some("hunter2".into());
        pw.password = true;
        let text = render(&[pw], None, 0, &Caps::new(None, None));
        assert!(!text.contains("hunter2"));
        assert!(text.contains("[0] edit \"Password\" password"));
    }

    #[test]
    fn caps_and_cuts_are_reported() {
        let nodes: Vec<Node> = (0..2000)
            .map(|i| node(1, 50020, &format!("item {i} {}", "x".repeat(100))))
            .collect();
        let caps = Caps::new(None, None);
        assert_eq!(caps.bytes, DEFAULT_BYTES);
        assert_eq!(Caps::new(None, Some(600)).bytes, MAX_BYTES);
        let text = render(&nodes, Some(Cut::Nodes), 3, &caps);
        assert!(text.len() <= caps.bytes + 300);
        assert!(text.contains("more elements not shown"));
        assert!(text.contains("3 offscreen elements"));
        assert!(text.contains(&format!("stopped at {DEFAULT_NODES} elements")));
        let text = render(&nodes[..2], Some(Cut::Time), 0, &caps);
        assert!(text.contains("slow to answer"));
        assert!(!text.contains("offscreen"));
        let long = node(0, 50020, &"y".repeat(500));
        assert_eq!(
            line(0, &long, 0, false).chars().count(),
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

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

/// Which elements a tree shows (`find`, `types`). Empty: all of them. The walk is the same either way; a match is
/// shown under its shown ancestors, so the outline still says where it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    /// Lowercased text to find in a name, value or automation id.
    pub find: Option<String>,
    /// Control type names (`button`, `edit` …), as the outline writes them.
    pub types: Vec<String>,
}

/// Longest `find` text kept.
const MAX_FIND: usize = 100;

impl Filter {
    /// From the tool's optional `find` and `types`. Unknown type names are an error that lists the known ones.
    pub fn new(find: Option<&str>, types: &[&str]) -> Result<Self, String> {
        let find = find
            .map(|f| {
                f.trim()
                    .to_lowercase()
                    .chars()
                    .take(MAX_FIND)
                    .collect::<String>()
            })
            .filter(|f| !f.is_empty());
        let mut known = Vec::new();
        for t in types {
            let t = t.trim().to_lowercase();
            if !CONTROL_TYPES.contains(&t.as_str()) {
                return Err(format!(
                    "unknown control type \"{t}\"; types are: {}",
                    CONTROL_TYPES.join(", ")
                ));
            }
            if !known.contains(&t) {
                known.push(t);
            }
        }
        Ok(Self { find, types: known })
    }

    pub fn is_empty(&self) -> bool {
        self.find.is_none() && self.types.is_empty()
    }

    fn matches(&self, n: &Node) -> bool {
        let typed = self.types.is_empty()
            || self
                .types
                .iter()
                .any(|t| t == control_type_name(n.control_type));
        let found = self.find.as_deref().is_none_or(|f| {
            let value = n.value.as_deref().filter(|_| !n.password).unwrap_or("");
            [n.name.as_str(), value, n.automation_id.as_str()]
                .iter()
                .any(|text| text.to_lowercase().contains(f))
        });
        typed && found
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
/// out. With a `filter`, only matching lines are written, each after those of its ancestors not written yet. Stops
/// at `caps.bytes` and says so.
pub fn render(
    nodes: &[Node],
    cut: Option<Cut>,
    offscreen: u32,
    caps: &Caps,
    filter: &Filter,
) -> String {
    let mut out = String::new();
    let mut hidden = 0usize;
    let mut matched = 0usize;
    // The current node's ancestors that get a line: (depth, ref, written yet).
    let mut shown: Vec<(u32, usize, bool)> = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        while shown.last().is_some_and(|(d, _, _)| *d >= node.depth) {
            shown.pop();
        }
        let hit = !filter.is_empty() && filter.matches(node);
        if merged(node) && !hit {
            continue;
        }
        shown.push((node.depth, i, false));
        if !filter.is_empty() && !hit {
            continue;
        }
        matched += usize::from(hit);
        // This line, after any ancestor lines not written yet.
        for (level, (_, at, written)) in shown.iter_mut().enumerate() {
            if *written {
                continue;
            }
            *written = true;
            let at = *at;
            if hidden > 0 {
                hidden += 1;
                continue;
            }
            let line = line(at, &nodes[at], level, caps.bounds);
            if out.len() + line.len() + 1 > caps.bytes {
                hidden = 1;
                continue;
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    if !filter.is_empty() {
        out.push_str(&match matched {
            0 => format!(
                "… nothing matches among the {} elements walked.\n",
                nodes.len()
            ),
            n => format!("… {n} matching of the {} elements walked.\n", nodes.len()),
        });
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
    usize::try_from(id - 50000)
        .ok()
        .and_then(|i| NAMES.get(i))
        .copied()
        .unwrap_or("element")
}

/// Every name `control_type_name` gives (what `types` takes).
const CONTROL_TYPES: [&str; 42] = {
    let mut all = [""; 42];
    let mut i = 0;
    while i < NAMES.len() {
        all[i] = NAMES[i];
        i += 1;
    }
    all[41] = "element";
    all
};

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
        let text = render(&nodes, None, 0, &Caps::new(None, None), &Filter::default());
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
        let text = render(&nodes, None, 0, &caps, &Filter::default());
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
        let text = render(&nodes, None, 0, &Caps::new(None, None), &Filter::default());
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
        let text = render(&[pw], None, 0, &Caps::new(None, None), &Filter::default());
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
        let text = render(&nodes, Some(Cut::Nodes), 3, &caps, &Filter::default());
        assert!(text.len() <= caps.bytes + 300);
        assert!(text.contains("more elements not shown"));
        assert!(text.contains("3 offscreen elements"));
        assert!(text.contains(&format!("stopped at {DEFAULT_NODES} elements")));
        let text = render(&nodes[..2], Some(Cut::Time), 0, &caps, &Filter::default());
        assert!(text.contains("slow to answer"));
        assert!(!text.contains("offscreen"));
        let long = node(0, 50020, &"y".repeat(500));
        assert_eq!(
            line(0, &long, 0, false).chars().count(),
            "[0] text \"\"".len() + MAX_FIELD
        );
    }

    #[test]
    fn filters_show_matches_under_their_ancestors() {
        let mut search = node(3, 50004, "Search");
        search.value = Some("cats".into());
        let mut pw = node(3, 50004, "PIN");
        pw.value = Some("secret".into());
        pw.password = true;
        let mut id_only = node(2, 50033, "");
        id_only.automation_id = "SaveBar".into();
        let nodes = [
            node(0, 50032, "App"),      // [0]
            node(1, 50021, "Toolbar"),  // [1]
            node(2, 50033, ""),         // [2] merged
            node(3, 50000, "Save"),     // [3]
            node(3, 50000, "Open"),     // [4]
            search,                     // [5]
            node(1, 50026, "Files"),    // [6]
            node(2, 50007, "save.txt"), // [7]
            pw,                         // [8]
            id_only,                    // [9] unnamed pane, matched by its automation id
        ];
        let caps = Caps::new(None, None);
        let f = Filter::new(Some(" SAVE "), &[]).unwrap();
        let text = render(&nodes, None, 0, &caps, &f);
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "[0] window \"App\"",
                "  [1] toolbar \"Toolbar\"",
                "    [3] button \"Save\"",
                "  [6] group \"Files\"",
                "    [7] listitem \"save.txt\"",
                "    [9] pane #SaveBar",
                "… 3 matching of the 10 elements walked.",
            ]
        );
        let f = Filter::new(None, &["edit", "Edit"]).unwrap();
        assert_eq!(f.types, ["edit"]);
        let text = render(&nodes, None, 0, &caps, &f);
        assert!(text.contains("    [5] edit \"Search\" = \"cats\""));
        assert!(text.contains("[8] edit \"PIN\" password"));
        assert!(text.contains("2 matching"));
        let f = Filter::new(Some("cats"), &["edit"]).unwrap();
        assert!(render(&nodes, None, 0, &caps, &f).contains("1 matching"));
        let f = Filter::new(Some("secret"), &[]).unwrap();
        let text = render(&nodes, None, 0, &caps, &f);
        assert!(!text.contains("secret"), "a password's value never matches");
        assert!(text.contains("nothing matches among the 10"));
        assert!(Filter::new(None, &["buton"])
            .unwrap_err()
            .contains("button"));
        assert!(Filter::new(Some("  "), &[]).unwrap().is_empty());
    }

    #[test]
    fn control_types() {
        assert_eq!(control_type_name(50000), "button");
        assert_eq!(control_type_name(50033), "pane");
        assert_eq!(control_type_name(50040), "appbar");
        assert_eq!(control_type_name(50041), "element");
        assert_eq!(control_type_name(0), "element");
        assert_eq!(CONTROL_TYPES[0], "button");
        assert_eq!(CONTROL_TYPES[40], "appbar");
        assert_eq!(CONTROL_TYPES[41], "element");
    }
}

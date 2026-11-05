//! What an approval card shows beyond its one line (Phase 2.8b): a compact diff for `Edit` / `MultiEdit` / `Write`,
//! and the whole shell command wrapped at token boundaries. Pure and unit-tested, except `read_existing`, which reads
//! the file a `Write` would replace.
//!
//! Each request gets two copies: the card's, capped at `CARD_LINES` with one line of context, and the full one for
//! the command window, capped at `FULL_LINES`. Nothing is guessed: when the input was cut on its way to Cay (2.8a's
//! 16 KB per string), the preview says so and gives no counts.

use std::path::Path;

use serde_json::Value;

use super::describe;
use super::interact::PLAN_TOOL;
use super::patch::{self, Kind, Patch};
use crate::ipc::{ApprovalPreview, CommandPreview, DiffLine, DiffOp, DiffPreview, PlanPreview};

/// Lines on the card, after which it says "+n more lines". Keeps the card within the island's window (D2).
pub const CARD_LINES: usize = 6;
/// Lines in the command window's copy.
pub const FULL_LINES: usize = 2000;
/// Columns a shell command wraps to (the card's command block is this many `ch` wide).
pub const WRAP_COLS: usize = 48;
/// Characters kept per diff line; the card cuts the rest with an ellipsis anyway.
const MAX_LINE: usize = 400;
const CONTEXT_CARD: usize = 1;
const CONTEXT_FULL: usize = 3;
/// Above this many cells the line diff gives up on matching and shows old then new.
const MAX_CELLS: usize = 1_000_000;
/// `Write` diffs against the file on disk up to this size.
const MAX_EXISTING: u64 = 256 * 1024;
const SHELL_TOOLS: &[&str] = &["Bash", "PowerShell"];

/// The file a `Write` would replace (or, for a cut plan, its plan file), as Cay found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Existing {
    /// Not a `Write`, or no path.
    NotRead,
    /// No such file: `Write` creates it.
    Missing,
    Text(String),
    /// Too big, not UTF-8, or not a file.
    Unreadable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Previews {
    /// `None` for a plan: the island says "Plan ready" and the command window shows it.
    pub card: Option<ApprovalPreview>,
    pub full: ApprovalPreview,
}

/// Lines a transcript entry's diff keeps (2.10a): enough to read an edit without holding whole files per entry.
pub const TRANSCRIPT_LINES: usize = 200;

/// The command window's transcript copy of an edit's diff (2.10a): `Edit`, `MultiEdit`, `Write` or a Codex patch.
/// Built from the input alone (no file read), so a `Write` shows its new text with no note about the old file. Other
/// tools have none: a command's label already holds it, and a plan has its own view.
pub fn for_transcript(tool: &str, input: &Value, cwd: &str) -> Option<ApprovalPreview> {
    if tool == PLAN_TOOL || (SHELL_TOOLS.contains(&tool) && patch::of_input(input).is_none()) {
        return None;
    }
    match build(tool, input, cwd, &Existing::NotRead, &[])?.full {
        ApprovalPreview::Diff(mut d) => {
            if tool == "Write" {
                d.note = None;
            }
            if d.lines.len() > TRANSCRIPT_LINES {
                d.more_lines += (d.lines.len() - TRANSCRIPT_LINES) as u32;
                d.lines.truncate(TRANSCRIPT_LINES);
            }
            Some(ApprovalPreview::Diff(d))
        }
        _ => None,
    }
}

/// For a `Write`: the file it would overwrite. For an `ExitPlanMode` whose plan was cut (`cut` names `plan`): its
/// plan file, if it's Markdown. Bounded reads; everything else is `NotRead`.
pub fn read_existing(tool: &str, input: &Value, cwd: &str, cut: &[String]) -> Existing {
    if tool == PLAN_TOOL {
        let path = input.get("planFilePath").and_then(Value::as_str);
        return match path {
            Some(p)
                if cut.iter().any(|c| c == "plan") && p.to_ascii_lowercase().ends_with(".md") =>
            {
                read_bounded(Path::new(p))
            }
            _ => Existing::NotRead,
        };
    }
    let Some(path) = (tool == "Write")
        .then(|| input.get("file_path").and_then(Value::as_str))
        .flatten()
        .filter(|p| !p.is_empty())
    else {
        return Existing::NotRead;
    };
    read_bounded(&Path::new(cwd).join(path))
}

fn read_bounded(path: &Path) -> Existing {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Existing::Missing,
        Ok(m) if m.is_file() && m.len() <= MAX_EXISTING => std::fs::read_to_string(path)
            .map(Existing::Text)
            .unwrap_or(Existing::Unreadable),
        _ => Existing::Unreadable,
    }
}

/// The card's and the command window's previews, or `None` for tools without one. `cut` names the input fields
/// that were shortened on the way (`ask::Request::cut`).
pub fn build(
    tool: &str,
    input: &Value,
    cwd: &str,
    existing: &Existing,
    cut: &[String],
) -> Option<Previews> {
    let was_cut = |field: &str| cut.iter().any(|c| c == field);
    if tool == PLAN_TOOL {
        let plan = input.get("plan").and_then(Value::as_str).unwrap_or("");
        // A cut plan is taken whole from its file only if the file starts with what arrived.
        let (markdown, cut) = match existing {
            Existing::Text(file) if was_cut("plan") && file.starts_with(plan) => {
                (file.clone(), false)
            }
            _ => (plan.to_string(), was_cut("plan")),
        };
        if markdown.trim().is_empty() {
            return None;
        }
        return Some(Previews {
            card: None,
            full: ApprovalPreview::Plan(PlanPreview { markdown, cut }),
        });
    }
    // Codex's edits (2.9a): a patch, by name or by what its command holds.
    if let Some(p) = patch::of_input(input) {
        return Some(patch_previews(&p, cwd, was_cut("command")));
    }
    if SHELL_TOOLS.contains(&tool) {
        let command = input.get("command").and_then(Value::as_str)?;
        let lines = wrap_command(command, WRAP_COLS);
        if lines.is_empty() {
            return None;
        }
        let cut = was_cut("command");
        let command = |cap: usize| {
            ApprovalPreview::Command(CommandPreview {
                more_lines: lines.len().saturating_sub(cap) as u32,
                lines: lines.iter().take(cap).cloned().collect(),
                cut,
            })
        };
        return Some(Previews {
            card: Some(command(CARD_LINES)),
            full: command(FULL_LINES),
        });
    }
    let file = describe::relative(
        input.get("file_path").and_then(Value::as_str).unwrap_or(""),
        cwd,
    );
    let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or("");
    let (edits, note, cut): (Vec<(&str, &str)>, Option<&str>, bool) = match tool {
        "Edit" => (
            vec![(text("old_string"), text("new_string"))],
            input["replace_all"]
                .as_bool()
                .unwrap_or(false)
                .then_some("Every occurrence"),
            was_cut("old_string") || was_cut("new_string"),
        ),
        "MultiEdit" => {
            let edits: Vec<(&str, &str)> = input
                .get("edits")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|e| {
                            (
                                e["old_string"].as_str().unwrap_or(""),
                                e["new_string"].as_str().unwrap_or(""),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            (edits, None, was_cut("edits"))
        }
        "Write" => {
            let cut = was_cut("content");
            // Against a cut copy, the old file's tail would read as deleted: show the new text alone.
            match existing {
                Existing::Text(old) if !cut => (vec![(old.as_str(), text("content"))], None, false),
                Existing::Missing => (vec![("", text("content"))], Some("New file"), cut),
                _ => (
                    vec![("", text("content"))],
                    Some("Replaces the whole file"),
                    cut,
                ),
            }
        }
        _ => return None,
    };
    if edits.is_empty() {
        return None;
    }
    let ops: Vec<Vec<(DiffOp, &str)>> = edits.iter().map(|(a, b)| diff(a, b)).collect();
    let added = ops
        .iter()
        .flatten()
        .filter(|(op, _)| *op == DiffOp::Add)
        .count() as u32;
    let removed = ops
        .iter()
        .flatten()
        .filter(|(op, _)| *op == DiffOp::Del)
        .count() as u32;
    let diff_preview = |context: usize, cap: usize| {
        let mut lines: Vec<DiffLine> = Vec::new();
        for edit in &ops {
            if !lines.is_empty() {
                lines.push(gap());
            }
            lines.extend(hunks(edit, context));
        }
        ApprovalPreview::Diff(DiffPreview {
            file: file.clone(),
            more_lines: lines.len().saturating_sub(cap) as u32,
            lines: lines.into_iter().take(cap).collect(),
            added: (!cut).then_some(added),
            removed: (!cut).then_some(removed),
            cut,
            note: note.map(str::to_string),
        })
    };
    Some(Previews {
        card: Some(diff_preview(CONTEXT_CARD, CARD_LINES)),
        full: diff_preview(CONTEXT_FULL, FULL_LINES),
    })
}

/// A Codex patch as a diff: every file's sections in order with a gap between them, and a note naming the files when
/// there are several (the card has room for one file name).
fn patch_previews(p: &Patch, cwd: &str, cut: bool) -> Previews {
    let names: Vec<String> = p
        .files
        .iter()
        .map(|f| describe::relative(&f.path, cwd))
        .collect();
    let total = p.files.len() + p.more_files as usize;
    let note = if total > 1 {
        Some(describe::truncate(
            &format!("{total} files: {}", names.join(", ")),
            MAX_LINE,
        ))
    } else {
        let f = &p.files[0];
        match (f.kind, &f.move_to) {
            (Kind::Add, _) => Some("New file".to_string()),
            (Kind::Delete, _) => Some("Deletes the file".to_string()),
            (Kind::Update, Some(to)) => Some(format!("Moves to {}", describe::relative(to, cwd))),
            (Kind::Update, None) => None,
        }
    };
    let added: u32 = p.files.iter().map(|f| f.count(DiffOp::Add)).sum();
    let removed: u32 = p.files.iter().map(|f| f.count(DiffOp::Del)).sum();
    let diff_preview = |context: usize, cap: usize| {
        let mut lines: Vec<DiffLine> = Vec::new();
        for section in p.files.iter().flat_map(|f| &f.sections) {
            let ops: Vec<(DiffOp, &str)> =
                section.iter().map(|(op, t)| (*op, t.as_str())).collect();
            let hunk = hunks(&ops, context);
            if hunk.is_empty() {
                continue;
            }
            if !lines.is_empty() {
                lines.push(gap());
            }
            lines.extend(hunk);
        }
        ApprovalPreview::Diff(DiffPreview {
            file: names[0].clone(),
            more_lines: lines.len().saturating_sub(cap) as u32,
            lines: lines.into_iter().take(cap).collect(),
            added: (!cut).then_some(added),
            removed: (!cut).then_some(removed),
            cut,
            note: note.clone(),
        })
    };
    Previews {
        card: Some(diff_preview(CONTEXT_CARD, CARD_LINES)),
        full: diff_preview(CONTEXT_FULL, FULL_LINES),
    }
}

fn gap() -> DiffLine {
    DiffLine {
        op: DiffOp::Gap,
        text: String::new(),
    }
}

/// A line diff: unchanged lines as `Ctx`, then each change as its deletions followed by its additions.
fn diff<'a>(old: &'a str, new: &'a str) -> Vec<(DiffOp, &'a str)> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (mid_a, mid_b) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let mut out: Vec<(DiffOp, &str)> = a[..prefix].iter().map(|l| (DiffOp::Ctx, *l)).collect();
    if (mid_a.len() + 1) * (mid_b.len() + 1) <= MAX_CELLS {
        out.extend(lcs(mid_a, mid_b));
    } else {
        out.extend(mid_a.iter().map(|l| (DiffOp::Del, *l)));
        out.extend(mid_b.iter().map(|l| (DiffOp::Add, *l)));
    }
    out.extend(a[a.len() - suffix..].iter().map(|l| (DiffOp::Ctx, *l)));
    out
}

/// Longest common subsequence by table; deletions come before additions within a change.
fn lcs<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(DiffOp, &'a str)> {
    let w = b.len() + 1;
    let mut t = vec![0u32; (a.len() + 1) * w];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            t[i * w + j] = if a[i] == b[j] {
                t[(i + 1) * w + j + 1] + 1
            } else {
                t[(i + 1) * w + j].max(t[i * w + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(a.len() + b.len());
    let mut adds = Vec::new();
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.append(&mut adds);
            out.push((DiffOp::Ctx, a[i]));
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || t[i * w + j + 1] >= t[(i + 1) * w + j]) {
            adds.push((DiffOp::Add, b[j]));
            j += 1;
        } else {
            out.push((DiffOp::Del, a[i]));
            i += 1;
        }
    }
    out.append(&mut adds);
    out
}

/// The changes with `context` unchanged lines around each; longer unchanged runs between changes become one gap,
/// and those before the first or after the last change are left out.
fn hunks(ops: &[(DiffOp, &str)], context: usize) -> Vec<DiffLine> {
    let changed: Vec<usize> = (0..ops.len())
        .filter(|&i| ops[i].0 != DiffOp::Ctx)
        .collect();
    let (Some(&first), Some(&last)) = (changed.first(), changed.last()) else {
        return Vec::new();
    };
    let near = |i: usize| {
        changed
            .iter()
            .any(|&c| i + context >= c && i <= c + context)
    };
    let mut out = Vec::new();
    let mut skipped = false;
    for (i, (op, text)) in ops
        .iter()
        .enumerate()
        .take(last + context + 1)
        .skip(first.saturating_sub(context))
    {
        if *op == DiffOp::Ctx && !near(i) {
            skipped = true;
            continue;
        }
        if skipped {
            out.push(gap());
            skipped = false;
        }
        out.push(DiffLine {
            op: *op,
            text: describe::truncate(text, MAX_LINE),
        });
    }
    out
}

/// The command as lines of at most `cols` characters, broken between tokens (continuations indented two spaces).
/// A token longer than a line breaks after a `/`, `\`, `=`, `,`, `&` or `;` where it can, else at the column.
/// Nothing is left out; blank lines at either end are.
pub fn wrap_command(command: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(8);
    let mut out = Vec::new();
    for source in command.trim().lines() {
        let source = source.trim_end().replace('\t', "    ");
        let mut line = String::new();
        // Whether `line` has begun (it can start with an empty word: leading or doubled spaces are kept).
        let mut started = false;
        let mut first = true;
        for word in source.split(' ') {
            let width = if first { cols } else { cols - 2 };
            let len = line.chars().count() + usize::from(started);
            if len + word.chars().count() <= width {
                if started {
                    line.push(' ');
                }
                line.push_str(word);
                started = true;
                continue;
            }
            if started {
                out.push(indent(&line, first));
                first = false;
                line.clear();
            }
            let mut rest = word.to_string();
            loop {
                let width = if first { cols } else { cols - 2 };
                if rest.chars().count() <= width {
                    break;
                }
                let tail = rest.split_off(break_at(&rest, width));
                out.push(indent(&rest, first));
                first = false;
                rest = tail;
            }
            line = rest;
            started = true;
        }
        out.push(indent(&line, first));
    }
    out
}

fn indent(line: &str, first: bool) -> String {
    if first {
        line.to_string()
    } else {
        format!("  {line}")
    }
}

/// Byte index to split a too-long token at: after the last break character within `width` chars, else at `width`.
fn break_at(token: &str, width: usize) -> usize {
    let mut hard = token.len();
    let mut soft = None;
    for (n, (i, c)) in token.char_indices().enumerate() {
        if n == width {
            hard = i;
            break;
        }
        if n > 0 && matches!(c, '/' | '\\' | '=' | ',' | '&' | ';') {
            soft = Some(i + c.len_utf8());
        }
    }
    soft.filter(|&s| s < hard).unwrap_or(hard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CWD: &str = r"C:\Users\me\wpill";

    fn lines(p: &ApprovalPreview) -> Vec<String> {
        match p {
            ApprovalPreview::Diff(d) => d
                .lines
                .iter()
                .map(|l| {
                    let sign = match l.op {
                        DiffOp::Add => "+",
                        DiffOp::Del => "-",
                        DiffOp::Ctx => " ",
                        DiffOp::Gap => "…",
                    };
                    format!("{sign}{}", l.text)
                })
                .collect(),
            ApprovalPreview::Command(c) => c.lines.clone(),
            ApprovalPreview::Plan(p) => p.markdown.lines().map(str::to_string).collect(),
        }
    }

    fn diff_of(p: &ApprovalPreview) -> &DiffPreview {
        match p {
            ApprovalPreview::Diff(d) => d,
            other => panic!("not a diff: {other:?}"),
        }
    }

    #[test]
    fn the_transcript_gets_edits_only_capped_and_without_reading_files() {
        let cwd = r"C:\work";
        let write = json!({ "file_path": "new.txt", "content": "x
".repeat(500) });
        let Some(ApprovalPreview::Diff(d)) = for_transcript("Write", &write, cwd) else {
            panic!("a write's diff");
        };
        assert_eq!(d.note, None, "no claim about a file it didn't read");
        assert_eq!(d.lines.len(), TRANSCRIPT_LINES);
        assert_eq!(d.lines.len() as u32 + d.more_lines, 500);
        assert_eq!(d.added, Some(500));
        let edit = json!({ "file_path": "a.rs", "old_string": "a", "new_string": "b", "replace_all": true });
        let Some(ApprovalPreview::Diff(d)) = for_transcript("Edit", &edit, cwd) else {
            panic!("an edit's diff");
        };
        assert_eq!(d.note.as_deref(), Some("Every occurrence"));
        assert_eq!(
            for_transcript("Bash", &json!({ "command": "dir" }), cwd),
            None
        );
        assert_eq!(
            for_transcript("Read", &json!({ "file_path": "a.rs" }), cwd),
            None
        );
        assert_eq!(
            for_transcript(PLAN_TOOL, &json!({ "plan": "# Plan" }), cwd),
            None
        );
    }
    #[test]
    fn edit_diff_with_counts_and_context() {
        let input = json!({
            "file_path": r"C:\Users\me\wpill\src\hover\logic.rs",
            "old_string": "fn a() {\n    one();\n    two();\n}",
            "new_string": "fn a() {\n    one();\n    three();\n    four();\n}",
        });
        let p = build("Edit", &input, CWD, &Existing::NotRead, &[]).unwrap();
        let card = diff_of(p.card.as_ref().unwrap());
        assert_eq!(card.file, "src/hover/logic.rs");
        assert_eq!((card.added, card.removed), (Some(2), Some(1)));
        assert_eq!(
            lines(p.card.as_ref().unwrap()),
            [
                "     one();",
                "-    two();",
                "+    three();",
                "+    four();",
                " }"
            ]
        );
        assert_eq!(card.more_lines, 0);
        assert!(!card.cut);
    }

    #[test]
    fn card_caps_lines_and_counts_the_rest() {
        let new: String = (0..20).map(|i| format!("line {i}\n")).collect();
        let input = json!({ "file_path": "C:\\x\\a.txt", "content": new });
        let p = build("Write", &input, CWD, &Existing::Missing, &[]).unwrap();
        let card = diff_of(p.card.as_ref().unwrap());
        assert_eq!(card.lines.len(), CARD_LINES);
        assert_eq!(card.more_lines, 20 - CARD_LINES as u32);
        assert_eq!(card.note.as_deref(), Some("New file"));
        assert_eq!(card.added, Some(20));
        assert_eq!(diff_of(&p.full).lines.len(), 20);
        assert_eq!(diff_of(&p.full).more_lines, 0);
    }

    #[test]
    fn write_diffs_against_the_file_on_disk() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\n";
        let new = "a\nB\nc\nd\ne\nf\ng\nH\n";
        let input = json!({ "file_path": "C:\\x\\a.txt", "content": new });
        let p = build("Write", &input, CWD, &Existing::Text(old.into()), &[]).unwrap();
        assert_eq!(
            lines(p.card.as_ref().unwrap()),
            [" a", "-b", "+B", " c", "…", " g"],
            "the card has one line of context, a gap between changes"
        );
        assert_eq!(diff_of(p.card.as_ref().unwrap()).more_lines, 2, "-h and +H");
        let full = diff_of(&p.full);
        assert_eq!((full.added, full.removed), (Some(2), Some(2)));
        assert!(lines(&p.full).contains(&"+H".to_string()));
        assert_eq!(full.note, None);
    }

    #[test]
    fn cut_input_has_no_counts() {
        let input = json!({ "file_path": "C:\\x\\a.txt", "content": "x\ny" });
        let p = build(
            "Write",
            &input,
            CWD,
            &Existing::Text("old\n".into()),
            &["content".into()],
        )
        .unwrap();
        let card = diff_of(p.card.as_ref().unwrap());
        assert!(card.cut);
        assert_eq!((card.added, card.removed), (None, None));
        assert_eq!(
            lines(p.card.as_ref().unwrap()),
            ["+x", "+y"],
            "no false deletions"
        );
        let edit = json!({ "file_path": "a", "old_string": "a", "new_string": "b" });
        let p = build(
            "Edit",
            &edit,
            CWD,
            &Existing::NotRead,
            &["new_string".into()],
        )
        .unwrap();
        assert_eq!(diff_of(p.card.as_ref().unwrap()).added, None);
    }

    #[test]
    fn multi_edit_and_replace_all() {
        let input = json!({ "file_path": "a.rs", "edits": [
            { "old_string": "x", "new_string": "y" },
            { "old_string": "p\nq", "new_string": "p" },
        ]});
        let p = build("MultiEdit", &input, CWD, &Existing::NotRead, &[]).unwrap();
        assert_eq!(
            lines(p.card.as_ref().unwrap()),
            ["-x", "+y", "…", " p", "-q"]
        );
        let all =
            json!({ "file_path": "a", "old_string": "a", "new_string": "b", "replace_all": true });
        let p = build("Edit", &all, CWD, &Existing::NotRead, &[]).unwrap();
        assert_eq!(
            diff_of(p.card.as_ref().unwrap()).note.as_deref(),
            Some("Every occurrence")
        );
    }

    #[test]
    fn huge_diffs_fall_back_to_old_then_new() {
        let old: String = (0..1500).map(|i| format!("o{i}\n")).collect();
        let new: String = (0..1500).map(|i| format!("n{i}\n")).collect();
        let ops = diff(&old, &new);
        assert_eq!(ops.len(), 3000);
        assert_eq!(ops[0], (DiffOp::Del, "o0"));
        assert_eq!(ops[1500], (DiffOp::Add, "n0"));
    }

    #[test]
    fn other_tools_have_no_preview() {
        assert!(build(
            "Read",
            &json!({ "file_path": "a" }),
            CWD,
            &Existing::NotRead,
            &[]
        )
        .is_none());
        assert!(build(
            "Bash",
            &json!({ "command": "  " }),
            CWD,
            &Existing::NotRead,
            &[]
        )
        .is_none());
        assert!(build("WebFetch", &json!({}), CWD, &Existing::NotRead, &[]).is_none());
    }

    #[test]
    fn commands_wrap_between_tokens() {
        let cmd = "git commit -m \"phase2.8: wrap long commands\" && git push origin main --force-with-lease";
        let wrapped = wrap_command(cmd, 30);
        assert_eq!(
            wrapped,
            [
                "git commit -m \"phase2.8: wrap",
                "  long commands\" && git push",
                "  origin main",
                "  --force-with-lease",
            ]
        );
        for line in &wrapped {
            assert!(line.chars().count() <= 30, "{line}");
        }
        let words: Vec<&str> = wrapped.iter().flat_map(|l| l.split_whitespace()).collect();
        assert_eq!(words.join(" "), cmd, "nothing is cut or reordered");
        assert_eq!(wrap_command("ls", 30), ["ls"]);
        assert_eq!(wrap_command("echo a\necho b\n", 30), ["echo a", "echo b"]);
    }

    #[test]
    fn long_tokens_break_at_separators() {
        let wrapped = wrap_command(
            "cat C:/Users/me/Documents/GitHub/wpill/src-tauri/src/agent/preview.rs",
            24,
        );
        assert_eq!(wrapped[0], "cat");
        assert!(wrapped[1..].iter().all(|l| l.chars().count() <= 24));
        assert!(wrapped[1].ends_with('/'), "{wrapped:?}");
        assert_eq!(
            wrapped.concat().replace("  ", ""),
            "catC:/Users/me/Documents/GitHub/wpill/src-tauri/src/agent/preview.rs"
        );
        let hard = wrap_command(&"x".repeat(20), 8);
        assert_eq!(hard, ["xxxxxxxx", "  xxxxxx", "  xxxxxx"]);
    }

    #[test]
    fn shell_preview_caps_and_marks_cut() {
        let cmd = (0..10)
            .map(|i| format!("echo {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let p = build(
            "Bash",
            &json!({ "command": cmd }),
            CWD,
            &Existing::NotRead,
            &["command".into()],
        )
        .unwrap();
        let ApprovalPreview::Command(card) = p.card.as_ref().unwrap() else {
            panic!()
        };
        assert_eq!(card.lines.len(), CARD_LINES);
        assert_eq!(card.more_lines, 4);
        assert!(card.cut);
    }

    #[test]
    fn plans_are_whole_and_never_on_the_card() {
        let input = json!({ "plan": "# Plan\n1. a", "planFilePath": r"C:\p.md" });
        let p = build(PLAN_TOOL, &input, CWD, &Existing::NotRead, &[]).unwrap();
        assert_eq!(p.card, None);
        assert_eq!(
            p.full,
            ApprovalPreview::Plan(PlanPreview {
                markdown: "# Plan\n1. a".into(),
                cut: false
            })
        );
        let cut = ["plan".to_string()];
        let whole = Existing::Text("# Plan\n1. a\n2. b".into());
        let p = build(PLAN_TOOL, &input, CWD, &whole, &cut).unwrap();
        assert_eq!(
            p.full,
            ApprovalPreview::Plan(PlanPreview {
                markdown: "# Plan\n1. a\n2. b".into(),
                cut: false
            }),
            "taken whole from its file"
        );
        let other = Existing::Text("# Another plan".into());
        let p = build(PLAN_TOOL, &input, CWD, &other, &cut).unwrap();
        assert!(matches!(
            p.full,
            ApprovalPreview::Plan(PlanPreview { cut: true, .. })
        ));
        assert_eq!(
            read_existing(PLAN_TOOL, &input, CWD, &[]),
            Existing::NotRead,
            "not cut"
        );
        assert!(build(PLAN_TOOL, &json!({}), CWD, &Existing::NotRead, &[]).is_none());
    }

    #[test]
    fn reads_the_file_a_write_replaces() {
        let dir = std::env::temp_dir().join(format!("cay-preview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "old\n").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let write = |p: &str| json!({ "file_path": p, "content": "new" });
        assert_eq!(
            read_existing("Write", &write(&file.to_string_lossy()), &cwd, &[]),
            Existing::Text("old\n".into())
        );
        assert_eq!(
            read_existing("Write", &write("a.txt"), &cwd, &[]),
            Existing::Text("old\n".into()),
            "relative to the folder"
        );
        assert_eq!(
            read_existing("Write", &write("b.txt"), &cwd, &[]),
            Existing::Missing
        );
        assert_eq!(
            read_existing("Edit", &write("a.txt"), &cwd, &[]),
            Existing::NotRead
        );
        std::fs::write(&file, vec![b'x'; MAX_EXISTING as usize + 1]).unwrap();
        assert_eq!(
            read_existing("Write", &write("a.txt"), &cwd, &[]),
            Existing::Unreadable
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn codex_patches_show_as_a_diff() {
        let patch = [
            "*** Begin Patch",
            "*** Update File: src/main.rs",
            "@@",
            " fn main() {",
            "-    old();",
            "+    new();",
            " }",
            "*** Add File: notes.txt",
            "+hi",
            "*** End Patch",
        ]
        .join("\n");
        let input = json!({ "command": patch });
        let p = build("apply_patch", &input, CWD, &Existing::NotRead, &[]).unwrap();
        let card = diff_of(p.card.as_ref().unwrap());
        assert_eq!(card.file, "src/main.rs");
        assert_eq!((card.added, card.removed), (Some(2), Some(1)));
        assert_eq!(
            card.note.as_deref(),
            Some("2 files: src/main.rs, notes.txt")
        );
        assert_eq!(
            lines(p.card.as_ref().unwrap()),
            vec![
                " fn main() {",
                "-    old();",
                "+    new();",
                " }",
                "…",
                "+hi"
            ]
        );
        // Through the shell, as Codex sometimes sends it; a cut command has no counts.
        let shell = json!({ "command": format!("apply_patch <<'EOF'\n{patch}\nEOF") });
        let p = build("Bash", &shell, CWD, &Existing::NotRead, &["command".into()]).unwrap();
        let card = diff_of(p.card.as_ref().unwrap());
        assert_eq!((card.added, card.removed, card.cut), (None, None, true));
        assert_eq!(
            describe::summary("apply_patch", &input, CWD),
            "src/main.rs +1"
        );
        assert_eq!(
            describe::step_line("apply_patch", &input, CWD),
            "Edit · src/main.rs +1"
        );
    }
}

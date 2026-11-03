//! Codex's `apply_patch` format (2.9a), read for the approval card: which files a patch touches and the lines it
//! changes. Pure; unit-tested.
//!
//! ```text
//! *** Begin Patch
//! *** Update File: src/main.rs
//! *** Move to: src/app.rs
//! @@ fn main() {
//!  context
//! -old line
//! +new line
//! *** Add File: notes.txt
//! +hello
//! *** Delete File: old.txt
//! *** End Patch
//! ```
//!
//! The text is the agent's, so nothing here trusts it: a patch that doesn't parse is shown as a command instead.

use serde_json::Value;

use crate::ipc::DiffOp;

const BEGIN: &str = "*** Begin Patch";
const END: &str = "*** End Patch";
/// At most this many files are read from one patch; the rest are counted.
const MAX_FILES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Add,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FilePatch {
    pub path: String,
    pub kind: Kind,
    /// `*** Move to:` for an update that renames the file.
    pub move_to: Option<String>,
    /// The changed lines with their context; each `@@` starts a new section.
    pub sections: Vec<Vec<(DiffOp, String)>>,
}

impl FilePatch {
    pub fn count(&self, op: DiffOp) -> u32 {
        self.sections
            .iter()
            .flatten()
            .filter(|(o, _)| *o == op)
            .count() as u32
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    pub files: Vec<FilePatch>,
    /// Files beyond `MAX_FILES`.
    pub more_files: u32,
}

/// The patch in a tool input: its `command` (how Codex's hooks pass it), `patch` or `input`, if one of them is a patch,
/// directly or as `apply_patch <<'EOF'` through the shell.
pub fn of_input(input: &Value) -> Option<Patch> {
    ["command", "patch", "input"]
        .iter()
        .filter_map(|key| input.get(key).and_then(Value::as_str))
        .find_map(|text| parse(text).or_else(|| parse(through_shell(text)?)))
}

/// The patch after `apply_patch` on a shell command's first line, if there is one.
fn through_shell(command: &str) -> Option<&str> {
    let rest = command.trim_start().strip_prefix("apply_patch")?;
    let start = rest.find(BEGIN)?;
    (rest[..start].lines().count() <= 1).then_some(&rest[start..])
}

/// `None` unless `text` starts with `*** Begin Patch` and names at least one file.
pub fn parse(text: &str) -> Option<Patch> {
    let mut lines = text.trim_start().lines();
    if lines.next()?.trim_end() != BEGIN {
        return None;
    }
    let mut files: Vec<FilePatch> = Vec::new();
    let mut more_files = 0u32;
    // Lines of a file past `MAX_FILES` are skipped, not parsed into the last kept file.
    let mut skipping = false;
    for line in lines {
        let header = |prefix: &str| line.strip_prefix(prefix).map(|p| p.trim().to_string());
        let new_file = header("*** Add File: ")
            .map(|p| (p, Kind::Add))
            .or_else(|| header("*** Update File: ").map(|p| (p, Kind::Update)))
            .or_else(|| header("*** Delete File: ").map(|p| (p, Kind::Delete)));
        if let Some((path, kind)) = new_file {
            if path.is_empty() {
                return None;
            }
            if files.len() >= MAX_FILES {
                more_files += 1;
                skipping = true;
                continue;
            }
            skipping = false;
            files.push(FilePatch {
                path,
                kind,
                move_to: None,
                sections: Vec::new(),
            });
            continue;
        }
        if line.trim_end() == END {
            break;
        }
        if skipping {
            continue;
        }
        let Some(file) = files.last_mut() else {
            // Anything before the first file header isn't a patch we understand.
            if line.trim().is_empty() {
                continue;
            }
            return None;
        };
        if let Some(to) = header("*** Move to: ") {
            file.move_to = Some(to);
            continue;
        }
        if line.starts_with("@@") {
            file.sections.push(Vec::new());
            continue;
        }
        if line.starts_with("*** ") {
            // `*** End of File` and anything newer: no line of its own.
            continue;
        }
        let (op, rest) = match line.chars().next() {
            Some('+') => (DiffOp::Add, &line[1..]),
            Some('-') => (DiffOp::Del, &line[1..]),
            Some(' ') => (DiffOp::Ctx, &line[1..]),
            None => (DiffOp::Ctx, ""),
            // A line without a marker: context, as Codex reads it.
            Some(_) => (DiffOp::Ctx, line),
        };
        if file.sections.is_empty() {
            file.sections.push(Vec::new());
        }
        if let Some(section) = file.sections.last_mut() {
            section.push((op, rest.to_string()));
        }
    }
    if files.is_empty() {
        return None;
    }
    for file in &mut files {
        file.sections.retain(|s| !s.is_empty());
    }
    Some(Patch { files, more_files })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PATCH: &str = "*** Begin Patch
*** Update File: src/main.rs
@@ fn main() {
 fn main() {
-    println!(\"hi\");
+    println!(\"hello\");
 }
*** Add File: notes.txt
+first
+second
*** Delete File: old.txt
*** Update File: a.rs
*** Move to: b.rs
@@
-x
+y
*** End of File
*** End Patch";

    #[test]
    fn reads_every_kind_of_file() {
        let p = parse(PATCH).unwrap();
        assert_eq!(p.files.len(), 4);
        let main = &p.files[0];
        assert_eq!(
            (main.path.as_str(), main.kind),
            ("src/main.rs", Kind::Update)
        );
        assert_eq!(
            main.sections,
            vec![vec![
                (DiffOp::Ctx, "fn main() {".to_string()),
                (DiffOp::Del, "    println!(\"hi\");".to_string()),
                (DiffOp::Add, "    println!(\"hello\");".to_string()),
                (DiffOp::Ctx, "}".to_string()),
            ]]
        );
        assert_eq!(
            (p.files[1].kind, p.files[1].count(DiffOp::Add)),
            (Kind::Add, 2)
        );
        assert_eq!(
            (p.files[2].kind, p.files[2].sections.len()),
            (Kind::Delete, 0)
        );
        assert_eq!(p.files[3].move_to.as_deref(), Some("b.rs"));
        assert_eq!(p.files[3].count(DiffOp::Del), 1);
        assert_eq!(p.more_files, 0);
    }

    #[test]
    fn finds_the_patch_in_a_tool_input() {
        assert!(of_input(&json!({ "command": PATCH })).is_some());
        assert!(of_input(&json!({ "command": "git status" })).is_none());
        let heredoc = format!("apply_patch <<'EOF'\n{PATCH}\nEOF");
        assert_eq!(
            of_input(&json!({ "command": heredoc }))
                .unwrap()
                .files
                .len(),
            4
        );
        // Only on the command's first line: a patch quoted further down a script isn't one.
        let script = format!("echo hi\napply_patch <<'EOF'\n{PATCH}");
        assert!(of_input(&json!({ "command": script })).is_none());
    }

    #[test]
    fn refuses_what_isnt_a_patch() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("*** Begin Patch\n*** End Patch"), None);
        assert_eq!(parse("echo hi\n*** Begin Patch"), None);
        assert_eq!(parse("*** Begin Patch\n+orphan line\n*** End Patch"), None);
        assert_eq!(parse("*** Begin Patch\n*** Add File: \n+x"), None);
    }

    #[test]
    fn caps_the_files_read() {
        let mut text = String::from("*** Begin Patch\n");
        for i in 0..MAX_FILES + 3 {
            text.push_str(&format!("*** Add File: f{i}.txt\n+line\n"));
        }
        let p = parse(&text).unwrap();
        assert_eq!(p.files.len(), MAX_FILES);
        assert_eq!(p.more_files, 3);
        assert!(p.files.iter().all(|f| f.count(DiffOp::Add) == 1));
    }
}

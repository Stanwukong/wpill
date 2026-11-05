//! Attachments for a message (2.10b, D52): which files are images, the caps, the names their copies get, and what
//! Claude Code receives. Pure; the copying and scaling are in `attach.rs`.
//!
//! Images go to Claude Code as image content blocks in the stream-json user message, scaled like desktop looks.
//! Other files are copied into the run's own `attachments` folder, which the run can read (`--add-dir`), and the
//! message lists their paths. Claude only ever sees copies; nothing outside the project folder is opened up.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::ipc::{AgentAttachment, AttachmentKind};

/// At most this many images per message.
pub const MAX_IMAGES: usize = 4;
/// At most this many other files per message.
pub const MAX_FILES: usize = 10;
/// Each file (or image, before scaling) at most this big.
pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
/// All of one message's attachments together at most this big.
pub const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;
/// The folder under the run's own folder that holds the copies.
pub const FOLDER: &str = "attachments";
/// Longest name a copy gets (its extension included).
const MAX_NAME: usize = 120;

/// A file you picked, before it's copied: its name and size as read from disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub bytes: u64,
}

/// Images by extension (PNG, JPEG, GIF, WebP); anything else is a file. A file that only claims to be an image
/// fails to decode and the message isn't sent.
pub fn kind_of(name: &str) -> AttachmentKind {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" => AttachmentKind::Image,
        _ => AttachmentKind::File,
    }
}

/// Why a message's attachments can't be sent, in words for the command window.
pub fn check(candidates: &[Candidate]) -> Result<(), String> {
    let images = candidates
        .iter()
        .filter(|c| kind_of(&c.name) == AttachmentKind::Image)
        .count();
    let files = candidates.len() - images;
    if images > MAX_IMAGES {
        return Err(format!("A message can carry at most {MAX_IMAGES} images."));
    }
    if files > MAX_FILES {
        return Err(format!("A message can carry at most {MAX_FILES} files."));
    }
    if let Some(big) = candidates.iter().find(|c| c.bytes > MAX_FILE_BYTES) {
        return Err(format!(
            "{} is over {} MB, the most one attachment can be.",
            big.name,
            MAX_FILE_BYTES / (1024 * 1024)
        ));
    }
    let total: u64 = candidates.iter().map(|c| c.bytes).sum();
    if total > MAX_TOTAL_BYTES {
        return Err(format!(
            "These attachments are over {} MB together, the most one message can carry.",
            MAX_TOTAL_BYTES / (1024 * 1024)
        ));
    }
    Ok(())
}

/// The name a copy gets in the attachments folder: no path or reserved characters, no Windows device names, not too
/// long, and not one already `taken` (compared without case, as Windows does): `notes (2).txt`.
pub fn copy_name(original: &str, taken: &HashSet<String>) -> String {
    let cleaned: String = original
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    let (stem, ext) = match cleaned.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !e.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (cleaned.clone(), String::new()),
    };
    let mut stem = if stem.is_empty() || is_device_name(&stem) {
        format!("file{}", if stem.is_empty() { "" } else { "_" }) + &stem
    } else {
        stem
    };
    let room = MAX_NAME.saturating_sub(ext.chars().count()).max(1);
    if stem.chars().count() > room {
        stem = stem.chars().take(room).collect();
    }
    let ext: String = ext.chars().take(MAX_NAME).collect();
    let mut name = format!("{stem}{ext}");
    let mut n = 2;
    while taken.contains(&name.to_lowercase()) {
        name = format!("{stem} ({n}){ext}");
        n += 1;
    }
    name
}

fn is_device_name(stem: &str) -> bool {
    let base = stem
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit())
}

/// The message's text with its files listed after it, so Claude knows where the copies are.
pub fn prompt_text(text: &str, files: &[(String, String)]) -> String {
    if files.is_empty() {
        return text.to_string();
    }
    let mut out = text.to_string();
    out.push_str(
        "\n\nAttached files (copies made by Cay in a folder you can read; the originals stay where they are):",
    );
    for (name, path) in files {
        out.push_str(&format!("\n- {name}: {path}"));
    }
    out
}

/// One scaled image for the message.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageBlock {
    pub mime: &'static str,
    pub base64: String,
}

/// A user message's `content` for Claude Code: the text alone, or the images first, then the text.
pub fn content(text: &str, images: &[ImageBlock]) -> Value {
    if images.is_empty() {
        return Value::String(text.to_string());
    }
    let mut blocks: Vec<Value> = images
        .iter()
        .map(|i| {
            json!({
                "type": "image",
                "source": { "type": "base64", "media_type": i.mime, "data": i.base64 },
            })
        })
        .collect();
    blocks.push(json!({ "type": "text", "text": text }));
    Value::Array(blocks)
}

/// What the audit records about a message's attachments: names, kinds and sizes, never their content or paths.
pub fn for_audit(attachments: &[AgentAttachment]) -> Value {
    Value::Array(
        attachments
            .iter()
            .map(|a| json!({ "name": a.name, "kind": a.kind, "bytes": a.bytes }))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, bytes: u64) -> Candidate {
        Candidate {
            name: name.into(),
            bytes,
        }
    }

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_of("shot.PNG"), AttachmentKind::Image);
        assert_eq!(kind_of("a.b.jpeg"), AttachmentKind::Image);
        assert_eq!(kind_of("anim.gif"), AttachmentKind::Image);
        assert_eq!(kind_of("p.webp"), AttachmentKind::Image);
        assert_eq!(kind_of("scan.bmp"), AttachmentKind::File);
        assert_eq!(kind_of("notes.txt"), AttachmentKind::File);
        assert_eq!(kind_of("png"), AttachmentKind::File);
    }

    #[test]
    fn caps() {
        assert!(check(&[]).is_ok());
        let four: Vec<_> = (0..4).map(|i| c(&format!("{i}.png"), 10)).collect();
        assert!(check(&four).is_ok());
        let five: Vec<_> = (0..5).map(|i| c(&format!("{i}.png"), 10)).collect();
        assert!(check(&five).unwrap_err().contains("4 images"));
        let ten: Vec<_> = (0..10).map(|i| c(&format!("{i}.txt"), 10)).collect();
        assert!(check(&ten).is_ok());
        let eleven: Vec<_> = (0..11).map(|i| c(&format!("{i}.txt"), 10)).collect();
        assert!(check(&eleven).unwrap_err().contains("10 files"));
        assert!(check(&[c("big.zip", MAX_FILE_BYTES)]).is_ok());
        assert!(check(&[c("big.zip", MAX_FILE_BYTES + 1)])
            .unwrap_err()
            .starts_with("big.zip"));
        let three = [
            c("a.zip", 20 << 20),
            c("b.zip", 20 << 20),
            c("c.zip", 11 << 20),
        ];
        assert!(check(&three).unwrap_err().contains("together"));
    }

    #[test]
    fn copy_names() {
        let mut taken = HashSet::new();
        assert_eq!(copy_name("notes.txt", &taken), "notes.txt");
        taken.insert("notes.txt".to_string());
        assert_eq!(copy_name("Notes.TXT", &taken), "Notes (2).TXT");
        taken.insert("notes (2).txt".to_string());
        assert_eq!(copy_name("notes.txt", &taken), "notes (3).txt");
        assert_eq!(copy_name("a:b?.md", &HashSet::new()), "a_b_.md");
        assert_eq!(copy_name("..\\up.txt", &HashSet::new()), ".._up.txt");
        assert_eq!(copy_name("CON.txt", &HashSet::new()), "file_CON.txt");
        assert_eq!(copy_name("com1", &HashSet::new()), "file_com1");
        assert_eq!(copy_name("console.txt", &HashSet::new()), "console.txt");
        assert_eq!(copy_name("trailing. ", &HashSet::new()), "trailing");
        assert_eq!(copy_name("", &HashSet::new()), "file");
        assert_eq!(copy_name(".env", &HashSet::new()), ".env");
        let long = format!("{}.txt", "x".repeat(300));
        let name = copy_name(&long, &HashSet::new());
        assert_eq!(name.chars().count(), MAX_NAME);
        assert!(name.ends_with(".txt"));
    }

    #[test]
    fn prompt_lists_files() {
        assert_eq!(prompt_text("hi", &[]), "hi");
        let text = prompt_text(
            "look",
            &[("a.txt".into(), "C:\\runs\\r\\attachments\\a.txt".into())],
        );
        assert!(text.starts_with("look\n\nAttached files"));
        assert!(text.ends_with("\n- a.txt: C:\\runs\\r\\attachments\\a.txt"));
    }

    #[test]
    fn content_blocks() {
        assert_eq!(content("hi", &[]), json!("hi"));
        let v = content(
            "what's this?",
            &[ImageBlock {
                mime: "image/png",
                base64: "AAAA".into(),
            }],
        );
        assert_eq!(v[0]["type"], "image");
        assert_eq!(v[0]["source"]["type"], "base64");
        assert_eq!(v[0]["source"]["media_type"], "image/png");
        assert_eq!(v[0]["source"]["data"], "AAAA");
        assert_eq!(v[1], json!({ "type": "text", "text": "what's this?" }));
    }

    #[test]
    fn audit_has_no_content_or_paths() {
        let v = for_audit(&[AgentAttachment {
            name: "a.png".into(),
            kind: AttachmentKind::Image,
            bytes: 12,
        }]);
        assert_eq!(
            v,
            json!([{ "name": "a.png", "kind": "image", "bytes": 12 }])
        );
    }
}

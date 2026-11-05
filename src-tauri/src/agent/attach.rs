//! Preparing a message's attachments (2.10b): images scaled like desktop looks, other files copied into the run's
//! own attachments folder. The rules (kinds, caps, names, content blocks) are in `attachments.rs`.
//!
//! Runs outside the agents lock (copies can take a moment). The run folder is deleted when its process exits, so
//! `restore` copies again whatever went missing before the message is written.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde_json::Value;

use super::attachments::{self, Candidate, ImageBlock};
use crate::ipc::{AgentAttachment, AttachmentKind};

/// A file copied for the run: where it came from and where the copy is.
#[derive(Debug, Clone)]
struct Copied {
    name: String,
    source: PathBuf,
    dest: PathBuf,
}

/// A message's attachments, ready to send.
#[derive(Debug, Clone, Default)]
pub struct Prepared {
    /// What the transcript and the audit show.
    pub list: Vec<AgentAttachment>,
    images: Vec<ImageBlock>,
    files: Vec<Copied>,
}

impl Prepared {
    /// The user message's `content`: the images, then the text with the copies' paths listed.
    pub fn content(&self, text: &str) -> Value {
        let files: Vec<(String, String)> = self
            .files
            .iter()
            .map(|f| (f.name.clone(), f.dest.to_string_lossy().into_owned()))
            .collect();
        attachments::content(&attachments::prompt_text(text, &files), &self.images)
    }

    /// Copies again any file whose copy is gone (the run's process exited and took its folder with it).
    pub fn restore(&self) -> Result<(), String> {
        for f in self.files.iter().filter(|f| !f.dest.exists()) {
            if let Some(parent) = f.dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::copy(&f.source, &f.dest)
                .map_err(|e| format!("Couldn't copy {}: {e}", f.name))?;
        }
        Ok(())
    }
}

/// Checks `paths` against the caps, scales the images and copies the other files into `folder` (the run's
/// attachments folder). Errors are sentences for the command window.
pub fn prepare(folder: &Path, paths: &[String]) -> Result<Prepared, String> {
    let mut found = Vec::with_capacity(paths.len());
    for raw in paths {
        let path = PathBuf::from(raw.trim());
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let meta = std::fs::metadata(&path)
            .map_err(|_| format!("{} isn't there any more.", shown(&name, raw)))?;
        if !meta.is_file() {
            return Err(format!(
                "{} is a folder; attach the files in it instead.",
                shown(&name, raw)
            ));
        }
        found.push((path, name, meta.len()));
    }
    let candidates: Vec<Candidate> = found
        .iter()
        .map(|(_, name, bytes)| Candidate {
            name: name.clone(),
            bytes: *bytes,
        })
        .collect();
    attachments::check(&candidates)?;

    let mut prepared = Prepared::default();
    let mut taken: HashSet<String> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_lowercase())
                .collect()
        })
        .unwrap_or_default();
    for (path, name, bytes) in found {
        let kind = attachments::kind_of(&name);
        match kind {
            AttachmentKind::Image => {
                let encoded = crate::desktop::scale_image(path.clone())
                    .map_err(|_| format!("{name} isn't an image Cay can read."))?;
                prepared.images.push(ImageBlock {
                    mime: encoded.mime,
                    base64: base64::engine::general_purpose::STANDARD.encode(&encoded.bytes),
                });
            }
            AttachmentKind::File => {
                std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
                let copy = attachments::copy_name(&name, &taken);
                taken.insert(copy.to_lowercase());
                let dest = folder.join(&copy);
                std::fs::copy(&path, &dest).map_err(|e| format!("Couldn't copy {name}: {e}"))?;
                prepared.files.push(Copied {
                    name: name.clone(),
                    source: path,
                    dest,
                });
            }
        }
        prepared.list.push(AgentAttachment { name, kind, bytes });
    }
    Ok(prepared)
}

fn shown<'a>(name: &'a str, raw: &'a str) -> &'a str {
    if name.is_empty() {
        raw
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cay-attach-{tag}-{}",
            super::super::audit::random_hex(4)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn files_are_copied_and_listed() {
        let src = scratch("src");
        let folder = scratch("dst").join("attachments");
        let a = src.join("notes.txt");
        std::fs::write(&a, "hello").unwrap();
        let sub = src.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let b = sub.join("notes.txt");
        std::fs::write(&b, "again").unwrap();

        let p = prepare(&folder, &[a.display().to_string(), b.display().to_string()]).unwrap();
        assert_eq!(p.list.len(), 2);
        assert_eq!(p.list[0].kind, AttachmentKind::File);
        assert_eq!(p.list[0].bytes, 5);
        assert_eq!(
            std::fs::read_to_string(folder.join("notes.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            std::fs::read_to_string(folder.join("notes (2).txt")).unwrap(),
            "again"
        );
        let content = p.content("read these");
        let text = content.as_str().unwrap();
        assert!(text.starts_with("read these\n\nAttached files"));
        assert!(text.contains(&folder.join("notes (2).txt").display().to_string()));

        // The process exited and its folder went: the copies come back before the message is written.
        std::fs::remove_dir_all(&folder).unwrap();
        p.restore().unwrap();
        assert!(folder.join("notes (2).txt").exists());

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(folder.parent().unwrap());
    }

    #[test]
    fn refusals() {
        let src = scratch("bad");
        let folder = src.join("attachments");
        let missing = src.join("gone.txt").display().to_string();
        assert!(prepare(&folder, &[missing])
            .unwrap_err()
            .contains("gone.txt"));
        assert!(prepare(&folder, &[src.display().to_string()])
            .unwrap_err()
            .contains("folder"));
        let fake = src.join("fake.png");
        std::fs::write(&fake, "not an image").unwrap();
        assert!(prepare(&folder, &[fake.display().to_string()])
            .unwrap_err()
            .contains("isn't an image"));
        assert!(!folder.exists(), "nothing was copied");
        let _ = std::fs::remove_dir_all(&src);
    }

    #[test]
    fn images_are_scaled_into_blocks() {
        // A 2x1 BMP written by hand: WIC decodes it, and it goes to Claude as PNG.
        let src = scratch("img");
        let path = src.join("dot.png");
        let mut bmp = Vec::new();
        let pixels: [u8; 8] = [0, 0, 255, 0, 255, 0, 0, 0];
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&(54u32 + 8).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&2i32.to_le_bytes());
        bmp.extend_from_slice(&1i32.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(&pixels);
        std::fs::write(&path, &bmp).unwrap();

        let p = prepare(&src.join("attachments"), &[path.display().to_string()]).unwrap();
        assert_eq!(p.list[0].kind, AttachmentKind::Image);
        let content = p.content("what colour?");
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[0]["source"]["media_type"], "image/png");
        let data = content[0]["source"]["data"].as_str().unwrap();
        let png = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        assert_eq!(content[1]["text"], "what colour?");
        let _ = std::fs::remove_dir_all(&src);
    }
}

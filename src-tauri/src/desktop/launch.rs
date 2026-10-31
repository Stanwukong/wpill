//! `launch` (Phase 2.7d): opens a link (`plan::launch_url` decides which) in its default app, or a Start-menu
//! app by name through `shell:AppsFolder\<AppUserModelID>`. No paths and no arguments: running programs or
//! scripts by path goes through the shell tool and its own approvals.

use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, SHGetKnownFolderItem,
    ShellExecuteExW, KF_FLAG_DEFAULT, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
    SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use super::plan::ActError;

/// A Start-menu app: its display name and AppUserModelID.
#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub name: String,
    pub id: String,
}

fn name_of(item: &IShellItem, kind: SIGDN) -> Option<String> {
    // SAFETY: the returned string is ours to free with CoTaskMemFree, after copying it.
    unsafe {
        let p: PWSTR = item.GetDisplayName(kind).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// Everything in the Start menu's All apps list. Needs COM on this thread.
pub fn apps() -> Vec<App> {
    let mut out = Vec::new();
    // SAFETY: COM calls on smart pointers released on drop; `fetched` describes how many items Next filled.
    unsafe {
        let Ok(folder) =
            SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
        else {
            return out;
        };
        let Ok(items) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else {
            return out;
        };
        loop {
            let mut item = [None];
            let mut fetched = 0u32;
            if items.Next(&mut item, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = item[0].take() else { break };
            if let (Some(name), Some(id)) = (
                name_of(&item, SIGDN_NORMALDISPLAY),
                name_of(&item, SIGDN_PARENTRELATIVEPARSING),
            ) {
                out.push(App { name, id });
            }
            if out.len() > 2000 {
                break;
            }
        }
    }
    out
}

/// The app called `name`: an exact match (any case), else the only one whose name contains it.
pub fn find<'a>(apps: &'a [App], name: &str) -> Result<&'a App, ActError> {
    let wanted = name.trim().to_lowercase();
    if wanted.is_empty() {
        return Err(ActError::NotFound("Say which app to open.".into()));
    }
    if let Some(app) = apps.iter().find(|a| a.name.to_lowercase() == wanted) {
        return Ok(app);
    }
    let close: Vec<&App> = apps
        .iter()
        .filter(|a| a.name.to_lowercase().contains(&wanted))
        .collect();
    match close.as_slice() {
        [one] => Ok(one),
        [] => Err(ActError::NotFound(format!(
            "No app called {name:?} is in the Start menu. Use its name as the Start menu shows it."
        ))),
        many => Err(ActError::NotFound(format!(
            "Several apps match {name:?}: {}. Use the exact name.",
            many.iter()
                .take(6)
                .map(|a| format!("{:?}", a.name))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn shell_open(file: &str) -> Result<(), ActError> {
    let file: Vec<u16> = file.encode_utf16().chain([0]).collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("open"),
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` and the string it points to outlive the call.
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| ActError::Failed(e.message().to_string()))
}

/// Opens an already-checked link.
pub fn open_url(url: &str) -> Result<(), ActError> {
    shell_open(url)
}

/// Starts a Start-menu app.
pub fn open_app(app: &App) -> Result<(), ActError> {
    shell_open(&format!("shell:AppsFolder\\{}", app.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finding_apps() {
        let apps: Vec<App> = [
            "Notepad",
            "Calculator",
            "Visual Studio Code",
            "Visual Studio Installer",
        ]
        .iter()
        .map(|n| App {
            name: n.to_string(),
            id: format!("id.{n}"),
        })
        .collect();
        assert_eq!(find(&apps, "notepad").unwrap().name, "Notepad");
        assert_eq!(find(&apps, "calc").unwrap().name, "Calculator");
        assert!(
            matches!(find(&apps, "visual studio"), Err(ActError::NotFound(m)) if m.contains("Several"))
        );
        assert!(matches!(find(&apps, "Paint"), Err(ActError::NotFound(_))));
        assert!(find(&apps, " ").is_err());
    }
}

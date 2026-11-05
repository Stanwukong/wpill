//! Pure: processes → app groups, sorted by memory, with Rust's protected list (Phase 3 design, option b).
//!
//! An app is every process with the same exe *path* (Chrome's, Edge's and VS Code's many processes share one), plus
//! helper processes (WebView2, crash handlers) folded into the app that started them, so Teams' and WhatsApp's
//! WebView2 processes count towards Teams and WhatsApp instead of one big "msedgewebview2" row. Other children keep
//! their own row: a VS Code terminal's shell is not VS Code.

use std::collections::HashMap;

/// A process with its exe path (when Cay could read it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub parent: u32,
    /// FILETIME ticks; with `pid`, what identifies this process (pids are reused).
    pub created: u64,
    pub session: u32,
    /// Image file name, e.g. `chrome.exe`.
    pub name: String,
    pub path: Option<String>,
    pub private_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// Lowercased exe path (or name, when the path couldn't be read).
    pub key: String,
    /// The exe name of the group's own processes (not its folded helpers).
    pub exe_name: String,
    pub path: Option<String>,
    /// `(pid, created)` of every member, the group's own processes first.
    pub members: Vec<(u32, u64)>,
    pub bytes: u64,
    /// Any member is protected: the whole group can't be ended.
    pub protected: bool,
}

/// Processes Cay never ends, whatever the UI shows (lowercase image names). Ending most of them crashes or locks
/// Windows; the rest are security software or parts of the desktop.
pub const PROTECTED: &[&str] = &[
    "system",
    "registry",
    "memory compression",
    "secure system",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "lsaiso.exe",
    "svchost.exe",
    "dwm.exe",
    "explorer.exe",
    "msmpeng.exe",
    "nissrv.exe",
    "securityhealthservice.exe",
    "audiodg.exe",
    "fontdrvhost.exe",
    "sihost.exe",
    "ctfmon.exe",
    "conhost.exe",
];

/// Helper processes that belong to whichever app started them (lowercase image names).
const HELPERS: &[&str] = &["msedgewebview2.exe", "crashpad_handler.exe"];

/// How far up a helper's parents to look for its app.
const MAX_DEPTH: usize = 16;

pub fn is_protected(p: &Proc, own_pid: u32) -> bool {
    p.pid == own_pid
        || p.pid == 0
        || p.pid == 4
        // Session 0 holds services and system processes, never a desktop app.
        || p.session == 0
        || PROTECTED.contains(&p.name.to_ascii_lowercase().as_str())
}

fn own_key(p: &Proc) -> String {
    p.path.as_deref().unwrap_or(&p.name).to_ascii_lowercase()
}

fn is_helper(p: &Proc) -> bool {
    HELPERS.contains(&p.name.to_ascii_lowercase().as_str())
}

/// Groups `procs` into apps, biggest first. Processes without a name (the idle process) are left out.
pub fn group(procs: &[Proc], own_pid: u32) -> Vec<Group> {
    let by_pid: HashMap<u32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
    // A parent pid only counts if that process is older than the child; otherwise the pid was reused.
    let parent_of = |p: &Proc| {
        by_pid
            .get(&p.parent)
            .copied()
            .filter(|q| q.pid != p.pid && q.created <= p.created)
    };
    let app_of = |p: &Proc| -> String {
        let mut at = p;
        for _ in 0..MAX_DEPTH {
            if !is_helper(at) {
                return own_key(at);
            }
            match parent_of(at) {
                Some(up) => at = up,
                None => break,
            }
        }
        own_key(p)
    };

    let mut groups: HashMap<String, Group> = HashMap::new();
    let mut helpers: Vec<(String, &Proc)> = Vec::new();
    for p in procs.iter().filter(|p| !p.name.is_empty()) {
        let key = app_of(p);
        if key != own_key(p) {
            helpers.push((key, p));
            continue;
        }
        let g = groups.entry(key.clone()).or_insert_with(|| Group {
            key,
            exe_name: p.name.clone(),
            path: p.path.clone(),
            members: Vec::new(),
            bytes: 0,
            protected: false,
        });
        g.members.push((p.pid, p.created));
        g.bytes += p.private_bytes;
        g.protected |= is_protected(p, own_pid);
    }
    for (key, p) in helpers {
        // The app it belongs to may have no row of its own (an unnamed parent): then it keeps its own.
        let key = if groups.contains_key(&key) {
            key
        } else {
            own_key(p)
        };
        let g = groups.entry(key.clone()).or_insert_with(|| Group {
            key,
            exe_name: p.name.clone(),
            path: p.path.clone(),
            members: Vec::new(),
            bytes: 0,
            protected: false,
        });
        g.members.push((p.pid, p.created));
        g.bytes += p.private_bytes;
        g.protected |= is_protected(p, own_pid);
    }

    let mut out: Vec<Group> = groups.into_values().collect();
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.key.cmp(&b.key)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: u32 = 9999;

    fn p(pid: u32, parent: u32, created: u64, name: &str, path: &str, mb: u64) -> Proc {
        Proc {
            pid,
            parent,
            created,
            session: 1,
            name: name.into(),
            path: (!path.is_empty()).then(|| path.into()),
            private_bytes: mb << 20,
        }
    }

    fn find<'a>(groups: &'a [Group], exe: &str) -> &'a Group {
        groups
            .iter()
            .find(|g| g.exe_name.eq_ignore_ascii_case(exe))
            .unwrap_or_else(|| panic!("{exe} missing"))
    }

    const CHROME: &str = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
    const WEBVIEW: &str =
        r"C:\Program Files (x86)\Microsoft\EdgeWebView\Application\msedgewebview2.exe";

    #[test]
    fn a_chrome_tree_is_one_app() {
        let procs = [
            p(100, 50, 10, "chrome.exe", CHROME, 200),
            p(101, 100, 11, "chrome.exe", CHROME, 300),
            p(102, 100, 12, "chrome.exe", CHROME, 100),
            p(
                103,
                100,
                13,
                "crashpad_handler.exe",
                r"C:\x\crashpad_handler.exe",
                5,
            ),
        ];
        let groups = group(&procs, OWN);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 4);
        assert_eq!(groups[0].bytes, 605 << 20);
        assert!(!groups[0].protected);
        // The app's own processes come before its helpers.
        assert_eq!(groups[0].members[3].0, 103);
    }

    #[test]
    fn webview2_counts_towards_the_app_that_started_it() {
        let teams = r"C:\Program Files\WindowsApps\MSTeams\ms-teams.exe";
        let whatsapp = r"C:\Program Files\WindowsApps\WhatsApp\WhatsApp.exe";
        let procs = [
            p(200, 50, 10, "ms-teams.exe", teams, 100),
            p(201, 200, 11, "msedgewebview2.exe", WEBVIEW, 400),
            p(202, 201, 12, "msedgewebview2.exe", WEBVIEW, 50),
            p(300, 50, 10, "WhatsApp.exe", whatsapp, 80),
            p(301, 300, 11, "msedgewebview2.exe", WEBVIEW, 220),
        ];
        let groups = group(&procs, OWN);
        assert_eq!(groups.len(), 2);
        assert_eq!(find(&groups, "ms-teams.exe").bytes, 550 << 20);
        assert_eq!(find(&groups, "whatsapp.exe").bytes, 300 << 20);
        assert_eq!(groups[0].exe_name, "ms-teams.exe");
    }

    #[test]
    fn a_helper_without_an_app_keeps_its_own_row() {
        let procs = [p(201, 4242, 11, "msedgewebview2.exe", WEBVIEW, 40)];
        let groups = group(&procs, OWN);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].exe_name, "msedgewebview2.exe");
    }

    #[test]
    fn a_reused_parent_pid_is_not_the_parent() {
        let teams = r"C:\x\ms-teams.exe";
        // pid 200 started *after* the helper, so it's a new process that reused its parent's pid.
        let procs = [
            p(200, 50, 99, "ms-teams.exe", teams, 100),
            p(201, 200, 11, "msedgewebview2.exe", WEBVIEW, 400),
        ];
        let groups = group(&procs, OWN);
        assert_eq!(groups.len(), 2);
        assert_eq!(find(&groups, "msedgewebview2.exe").bytes, 400 << 20);
    }

    #[test]
    fn other_children_are_their_own_apps() {
        let code = r"C:\Users\u\AppData\Local\Programs\Microsoft VS Code\Code.exe";
        let procs = [
            p(400, 50, 10, "Code.exe", code, 500),
            p(401, 400, 11, "Code.exe", code, 300),
            p(
                402,
                401,
                12,
                "powershell.exe",
                r"C:\Windows\System32\powershell.exe",
                60,
            ),
        ];
        let groups = group(&procs, OWN);
        assert_eq!(groups.len(), 2);
        assert_eq!(find(&groups, "code.exe").members.len(), 2);
        assert_eq!(find(&groups, "powershell.exe").members.len(), 1);
    }

    #[test]
    fn protected_groups_and_cay_itself() {
        let mut svc = p(
            600,
            500,
            10,
            "svchost.exe",
            r"C:\Windows\System32\svchost.exe",
            90,
        );
        svc.session = 0;
        let mut defender = p(
            700,
            500,
            10,
            "MsMpEng.exe",
            r"C:\ProgramData\Defender\MsMpEng.exe",
            250,
        );
        defender.session = 0;
        let mut service = p(
            800,
            500,
            10,
            "SomeService.exe",
            r"C:\svc\SomeService.exe",
            70,
        );
        service.session = 0;
        let procs = [
            svc,
            defender,
            service,
            p(900, 50, 10, "explorer.exe", r"C:\Windows\explorer.exe", 120),
            p(OWN, 50, 10, "cay.exe", r"C:\Cay\cay.exe", 60),
            p(OWN + 1, OWN, 11, "cay.exe", r"C:\Cay\cay.exe", 10),
            p(1000, 50, 10, "notepad.exe", r"C:\Windows\notepad.exe", 30),
        ];
        let groups = group(&procs, OWN);
        for exe in [
            "svchost.exe",
            "msmpeng.exe",
            "someservice.exe",
            "explorer.exe",
            "cay.exe",
        ] {
            assert!(find(&groups, exe).protected, "{exe} should be protected");
        }
        assert!(!find(&groups, "notepad.exe").protected);
    }

    #[test]
    fn sorted_biggest_first_and_unnamed_left_out() {
        let procs = [
            p(0, 0, 0, "", "", 1),
            p(10, 1, 1, "a.exe", r"C:\a.exe", 10),
            p(11, 1, 1, "b.exe", r"C:\b.exe", 30),
            p(12, 1, 1, "c.exe", "", 20),
        ];
        let groups = group(&procs, OWN);
        let order: Vec<&str> = groups.iter().map(|g| g.exe_name.as_str()).collect();
        assert_eq!(order, ["b.exe", "c.exe", "a.exe"]);
        // Without a path the name is the key.
        assert_eq!(groups[1].key, "c.exe");
    }
}

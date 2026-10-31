//! The pure parts of desktop actions (Phase 2.7d, CASESTUDY D27): what an action is, key chords, the text that
//! may be typed, coordinates for `SendInput`, which links may be opened, and the errors Claude sees. Unit-tested.

use std::time::Duration;

use super::logic::{LookError, Rect};

/// An action waits until you've let go of the mouse and keyboard for this long...
pub const QUIET: Duration = Duration::from_millis(500);
/// ...and gives up (aborted) if you're still busy after this.
pub const QUIET_MAX: Duration = Duration::from_secs(10);
/// Longest text one `set_value` may carry.
pub const MAX_TEXT: usize = 1000;
/// Longest text one `type_text` may carry: typing is paced (`input.rs`), about 125 ms a character at worst.
pub const MAX_TYPED: usize = 500;
/// Chords one `press_keys` may press, in order.
pub const MAX_CHORDS: usize = 5;
/// Units per `SendInput` batch; the target is checked again between batches.
pub const TYPE_BATCH: usize = 16;

/// A UI element as `ui_tree` described it, to find it again: identity first, position as a hint.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    pub runtime_id: Vec<i32>,
    pub control_type: i32,
    pub name: String,
    pub automation_id: String,
    pub rect: Rect,
}

/// What `invoke` does to an element. `Default` picks the element's main pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Default,
    Invoke,
    Toggle,
    Select,
    Expand,
    Collapse,
    ScrollIntoView,
    Focus,
}

impl Verb {
    pub fn parse(s: Option<&str>) -> Result<Verb, String> {
        Ok(match s.map(str::trim).unwrap_or("") {
            "" => Verb::Default,
            "invoke" => Verb::Invoke,
            "toggle" => Verb::Toggle,
            "select" => Verb::Select,
            "expand" => Verb::Expand,
            "collapse" => Verb::Collapse,
            "scrollIntoView" => Verb::ScrollIntoView,
            "focus" => Verb::Focus,
            other => {
                return Err(format!(
                    "unknown action {other:?}: use invoke, toggle, select, expand, collapse, \
                     scrollIntoView or focus"
                ))
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Double,
}

/// Where a click lands, resolved before acting.
#[derive(Debug, Clone, PartialEq)]
pub enum ClickAt {
    /// The centre of this element, found again first.
    Element(Element),
    /// Physical screen px.
    Point { x: i32, y: i32 },
    /// Image px in a screenshot of the target, taken when the window's frame was `frame`.
    Screenshot { x: i32, y: i32, view: View },
}

/// Where a screenshot sits on screen, kept so a click can be given in its image px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub origin: Rect,
    pub scale: f64,
    /// The window's frame when it was taken; if it has moved since, the view is stale.
    pub frame: Rect,
}

impl View {
    /// Image px → physical screen px.
    pub fn screen_point(self, x: i32, y: i32) -> (i32, i32) {
        let map = |o: i32, v: i32| o + (f64::from(v) * self.scale).round() as i32;
        (map(self.origin.x, x), map(self.origin.y, y))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LaunchTarget {
    Url(String),
    /// A Start-menu app by its display name.
    App(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Focus,
    Invoke { element: Element, verb: Verb },
    SetValue { element: Element, value: String },
    Click { at: ClickAt, button: Button },
    TypeText { text: String },
    PressKeys { chords: Vec<Chord> },
    Launch(LaunchTarget),
}

/// Why an action didn't run (or stopped), as Claude sees it: a stable code and what to do instead.
#[derive(Debug, Clone, PartialEq)]
pub enum ActError {
    WindowGone,
    ElementGone,
    StaleView,
    TopbarWindow,
    NotOnThisDesktop,
    NotResponding,
    Elevated,
    ScreenLocked,
    NotForeground,
    Covered,
    Disabled,
    Unsupported(String),
    PasswordField,
    Refused(String),
    NotFound(String),
    /// Stopped by your input (or Stop) after `done` of `total` steps.
    Aborted {
        done: usize,
        total: usize,
    },
    /// You were still using the mouse or keyboard when it was due to start.
    Busy,
    Failed(String),
}

impl ActError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::WindowGone => "windowGone",
            Self::ElementGone => "elementGone",
            Self::StaleView => "staleView",
            Self::TopbarWindow => "topbarWindow",
            Self::NotOnThisDesktop => "notOnThisDesktop",
            Self::NotResponding => "notResponding",
            Self::Elevated => "elevated",
            Self::ScreenLocked => "screenLocked",
            Self::NotForeground => "notForeground",
            Self::Covered => "covered",
            Self::Disabled => "disabled",
            Self::Unsupported(_) => "unsupported",
            Self::PasswordField => "passwordField",
            Self::Refused(_) => "refused",
            Self::NotFound(_) => "notFound",
            Self::Aborted { .. } | Self::Busy => "aborted",
            Self::Failed(_) => "failed",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::WindowGone => {
                "That window has closed (or the hwnd is stale). Call list_windows again.".into()
            }
            Self::ElementGone => "That element isn't there any more (or the ref is stale). Call \
                                  ui_tree again and use a ref from the new tree."
                .into(),
            Self::StaleView => "The window has moved or the screenshot is too old. Take a new \
                                screenshot (or use ui_tree refs) before clicking."
                .into(),
            Self::TopbarWindow => {
                "That's one of TopBar's own windows, which can't be acted on.".into()
            }
            Self::NotOnThisDesktop => {
                "That window isn't on the current virtual desktop, so it can't be acted on.".into()
            }
            Self::NotResponding => "That window isn't responding. Try again later.".into(),
            Self::Elevated => "That window belongs to an app running as administrator. Windows \
                               blocks input into it from TopBar, so it can't be acted on."
                .into(),
            Self::ScreenLocked => "The screen is locked or a Windows security prompt is showing, \
                                   so nothing was done."
                .into(),
            Self::NotForeground => {
                "The window couldn't be brought to the front (or something else \
                                    came in front), so nothing more was sent to it."
                    .into()
            }
            Self::Covered => "That point isn't on the window: something else is in front of it \
                              there, or it's outside the window. Look again."
                .into(),
            Self::Disabled => "That element is disabled.".into(),
            Self::Unsupported(why) => format!("Not possible on that element: {why}"),
            Self::PasswordField => {
                "That's a password field. TopBar never types or sets passwords; \
                                    ask the user to do it."
                    .into()
            }
            Self::Refused(why) => format!("Refused: {why}"),
            Self::NotFound(why) => why.clone(),
            Self::Aborted { done, total } => format!(
                "The user used the mouse or keyboard (or stopped the run) while TopBar was acting, \
                 so it stopped after {done} of {total} steps. Don't retry right away; say what you \
                 were doing and let the user decide."
            ),
            Self::Busy => "The user kept using the mouse or keyboard, so TopBar didn't act. Don't \
                           retry right away; say what you wanted to do."
                .into(),
            Self::Failed(why) => format!("The action failed: {why}"),
        }
    }
}

impl From<LookError> for ActError {
    fn from(e: LookError) -> Self {
        match e {
            LookError::WindowGone => Self::WindowGone,
            LookError::TopbarWindow => Self::TopbarWindow,
            LookError::NotOnThisDesktop => Self::NotOnThisDesktop,
            LookError::Minimized => Self::NotForeground,
            LookError::NotResponding => Self::NotResponding,
            LookError::Elevated => Self::Elevated,
            LookError::ScreenLocked => Self::ScreenLocked,
            LookError::Failed(why) => Self::Failed(why),
        }
    }
}

// ---- Keys -------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub vk: u16,
    pub name: &'static str,
    /// Sent with `KEYEVENTF_EXTENDEDKEY` (arrows, Home/End, Insert/Delete…).
    pub extended: bool,
}

const fn key(vk: u16, name: &'static str) -> Key {
    Key {
        vk,
        name,
        extended: false,
    }
}

const fn ext(vk: u16, name: &'static str) -> Key {
    Key {
        vk,
        name,
        extended: true,
    }
}

pub const CTRL: Key = key(0x11, "Ctrl");
pub const SHIFT: Key = key(0x10, "Shift");
pub const ALT: Key = key(0x12, "Alt");
pub const WIN: Key = ext(0x5B, "Win");
pub const ENTER: Key = key(0x0D, "Enter");
pub const TAB: Key = key(0x09, "Tab");

const NAMED: &[(&[&str], Key)] = &[
    (&["enter", "return"], ENTER),
    (&["tab"], TAB),
    (&["esc", "escape"], key(0x1B, "Esc")),
    (&["space"], key(0x20, "Space")),
    (&["backspace", "back"], key(0x08, "Backspace")),
    (&["delete", "del"], ext(0x2E, "Delete")),
    (&["insert", "ins"], ext(0x2D, "Insert")),
    (&["home"], ext(0x24, "Home")),
    (&["end"], ext(0x23, "End")),
    (&["pageup", "pgup"], ext(0x21, "PageUp")),
    (&["pagedown", "pgdn"], ext(0x22, "PageDown")),
    (&["left"], ext(0x25, "Left")),
    (&["up"], ext(0x26, "Up")),
    (&["right"], ext(0x27, "Right")),
    (&["down"], ext(0x28, "Down")),
    (&["menu", "apps", "contextmenu"], ext(0x5D, "Menu")),
    (&["plus", "="], key(0xBB, "Plus")),
    (&["minus", "-"], key(0xBD, "Minus")),
    (&["comma", ","], key(0xBC, "Comma")),
    (&["period", "."], key(0xBE, "Period")),
];

fn named(word: &str) -> Option<Key> {
    let lower = word.to_ascii_lowercase();
    if let Some((_, k)) = NAMED
        .iter()
        .find(|(names, _)| names.contains(&lower.as_str()))
    {
        return Some(*k);
    }
    // Letters and digits are their own virtual-key codes; F1–F24 start at 0x70.
    const LETTERS: [&str; 36] = [
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R",
        "S", "T", "U", "V", "W", "X", "Y", "Z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
    ];
    if word.len() == 1 {
        let c = word.chars().next()?.to_ascii_uppercase();
        let i = LETTERS.iter().position(|l| l.starts_with(c))?;
        return Some(key(c as u16, LETTERS[i]));
    }
    const F: [&str; 24] = [
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14",
        "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
    ];
    let upper = word.to_ascii_uppercase();
    let i = F.iter().position(|f| *f == upper)?;
    Some(key(0x70 + i as u16, F[i]))
}

/// One key chord: modifiers held, one key pressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    /// In Ctrl, Alt, Shift order.
    pub mods: Vec<Key>,
    pub key: Key,
}

impl Chord {
    /// "Ctrl+Shift+S".
    pub fn label(&self) -> String {
        let mut parts: Vec<&str> = self.mods.iter().map(|m| m.name).collect();
        parts.push(self.key.name);
        parts.join("+")
    }

    fn has(&self, m: Key) -> bool {
        self.mods.contains(&m)
    }
}

/// "ctrl+s", "Alt + F4", "Shift+Tab" → a chord, or why it can't be pressed. Win-key chords and the ones that
/// switch or leave the window (Alt+Tab, Ctrl+Esc, Ctrl+Shift+Esc, Ctrl+Alt+Delete) are refused: they act on
/// the shell, not on the window the user approved.
pub fn parse_chord(text: &str) -> Result<Chord, String> {
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    // "Ctrl++" means Ctrl+Plus.
    let parts: Vec<&str> = if text.trim_end().ends_with("++") {
        let mut p = parts[..parts.len() - 2].to_vec();
        p.push("plus");
        p
    } else {
        parts
    };
    if parts.iter().any(|p| p.is_empty()) {
        return Err(format!("{text:?} isn't a key chord (e.g. \"Ctrl+S\")"));
    }
    let (last, mods) = parts.split_last().ok_or("empty key chord")?;
    let mut held = Vec::new();
    for m in mods {
        let m = match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => CTRL,
            "shift" => SHIFT,
            "alt" => ALT,
            "win" | "windows" | "meta" | "super" | "cmd" => WIN,
            _ => return Err(format!("{m:?} isn't a modifier (Ctrl, Alt, Shift)")),
        };
        if !held.contains(&m) {
            held.push(m);
        }
    }
    let key = match last.to_ascii_lowercase().as_str() {
        "win" | "windows" | "meta" | "super" => WIN,
        "ctrl" | "control" | "shift" | "alt" => {
            return Err(format!("{text:?} has no key after the modifiers"))
        }
        _ => named(last).ok_or_else(|| format!("{last:?} isn't a key TopBar knows"))?,
    };
    held.sort_by_key(|m| [CTRL, ALT, SHIFT, WIN].iter().position(|o| o == m));
    let chord = Chord { mods: held, key };
    let esc = chord.key.vk == 0x1B;
    let blocked = chord.has(WIN)
        || chord.key == WIN
        || (chord.has(ALT) && (chord.key == TAB || esc))
        || (chord.has(CTRL) && esc)
        || (chord.has(CTRL) && chord.has(ALT) && chord.key.vk == 0x2E);
    if blocked {
        return Err(format!(
            "{} acts on Windows itself, not on the window; TopBar doesn't press it",
            chord.label()
        ));
    }
    Ok(chord)
}

/// What a chord does that the approval should spell out.
pub fn chord_note(chord: &Chord) -> Option<&'static str> {
    (chord.mods == [ALT] && chord.key.vk == 0x73).then_some("closes the window")
}

// ---- Text -------------------------------------------------------------------------------------------------

/// One step of typing: a UTF-16 unit (`KEYEVENTF_UNICODE`) or a key (Enter for a newline, Tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Char(u16),
    Key(Key),
}

/// How to type one character as a real key press, from `VkKeyScanExW`'s answer for the target's keyboard layout:
/// the virtual key and whether Shift is held. `None` (type it as `VK_PACKET` instead) when the layout has no key
/// for it or it needs Ctrl/Alt (AltGr). With Caps Lock on, Shift is inverted for letters.
pub fn key_for(scan: i16, caps_lock: bool) -> Option<(u16, bool)> {
    if scan == -1 {
        return None;
    }
    let vk = (scan as u16) & 0xff;
    let mods = ((scan as u16) >> 8) & 0xff;
    if mods & !1 != 0 {
        return None;
    }
    let letter = (0x41..=0x5A).contains(&vk);
    Some((vk, (mods & 1 != 0) ^ (caps_lock && letter)))
}

/// The text to type, as units. Refused: empty, too long, other control characters, or anything that looks
/// like a secret.
pub fn type_units(text: &str) -> Result<Vec<Unit>, ActError> {
    check_text(text, false)?;
    let chars = text.chars().count();
    if chars > MAX_TYPED {
        return Err(ActError::Refused(format!(
            "the text is {chars} characters; type_text takes at most {MAX_TYPED} (set_value takes {MAX_TEXT})"
        )));
    }
    let mut units = Vec::new();
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    for c in normalized.chars() {
        match c {
            '\n' => units.push(Unit::Key(ENTER)),
            '\t' => units.push(Unit::Key(TAB)),
            c => {
                let mut buf = [0u16; 2];
                units.extend(c.encode_utf16(&mut buf).iter().map(|u| Unit::Char(*u)));
            }
        }
    }
    Ok(units)
}

/// Length, control characters and secrets. `allow_empty` for `set_value` (clearing a field is fine).
pub fn check_text(text: &str, allow_empty: bool) -> Result<(), ActError> {
    let chars = text.chars().count();
    if chars == 0 && !allow_empty {
        return Err(ActError::Refused("there's no text to type".into()));
    }
    if chars > MAX_TEXT {
        return Err(ActError::Refused(format!(
            "the text is {chars} characters; at most {MAX_TEXT} per call"
        )));
    }
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(ActError::Refused(
            "the text has control characters; use press_keys for keys".into(),
        ));
    }
    if looks_secret(text) {
        return Err(ActError::Refused(
            "that text looks like a password, token or key, and TopBar never types secrets. Ask \
             the user to enter it themselves."
                .into(),
        ));
    }
    Ok(())
}

/// A heuristic: known token shapes, private keys, JWTs, and long random-looking words.
pub fn looks_secret(text: &str) -> bool {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY") {
        return true;
    }
    const PREFIXES: &[(&str, usize)] = &[
        ("sk-", 20),
        ("sk_live_", 16),
        ("rk_live_", 16),
        ("ghp_", 20),
        ("gho_", 20),
        ("ghu_", 20),
        ("ghs_", 20),
        ("ghr_", 20),
        ("github_pat_", 20),
        ("glpat-", 20),
        ("xoxb-", 15),
        ("xoxp-", 15),
        ("xoxa-", 15),
        ("xoxs-", 15),
        ("AIza", 35),
    ];
    text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ',' | ';'))
        .filter(|w| !w.is_empty())
        .any(|word| {
            if PREFIXES
                .iter()
                .any(|(p, min)| word.starts_with(p) && word.len() >= *min)
            {
                return true;
            }
            let alnum = |w: &str| w.chars().all(|c| c.is_ascii_alphanumeric());
            if (word.starts_with("AKIA") || word.starts_with("ASIA"))
                && word.len() == 20
                && alnum(word)
                && word.chars().all(|c| !c.is_ascii_lowercase())
            {
                return true;
            }
            let parts: Vec<&str> = word.split('.').collect();
            if word.starts_with("eyJ") && parts.len() == 3 && word.len() > 30 {
                return true;
            }
            random_looking(word)
        })
}

/// 24+ characters of letters, digits and `_-+=/`, with upper, lower and digits mixed and high entropy, and not
/// part of a URL or path.
fn random_looking(word: &str) -> bool {
    if word.len() < 24 || word.contains("://") || word.contains('.') || word.contains('\\') {
        return false;
    }
    if !word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '=' | '/'))
    {
        return false;
    }
    let has = |f: fn(&char) -> bool| word.chars().any(|c| f(&c));
    if !(has(char::is_ascii_uppercase)
        && has(char::is_ascii_lowercase)
        && has(char::is_ascii_digit))
    {
        return false;
    }
    let mut counts = [0u32; 128];
    for b in word.bytes() {
        counts[b as usize] += 1;
    }
    let n = word.len() as f64;
    let entropy: f64 = counts
        .iter()
        .filter(|c| **c > 0)
        .map(|c| {
            let p = f64::from(*c) / n;
            -p * p.log2()
        })
        .sum();
    entropy >= 4.0
}

// ---- Coordinates and links --------------------------------------------------------------------------------

/// Physical screen px → `SendInput`'s 0–65535 over the virtual desktop (`MOUSEEVENTF_VIRTUALDESK`). Windows
/// maps `n` back to `left + n·width/65536` (rounded down), so this picks the smallest `n` landing on `x`.
pub fn to_absolute(x: i32, y: i32, desk: Rect) -> (i32, i32) {
    let axis = |v: i32, origin: i32, size: i32| {
        let size = i64::from(size.max(1));
        let p = i64::from((v - origin).clamp(0, size as i32 - 1));
        ((p * 65536 + size - 1) / size).min(65535) as i32
    };
    (axis(x, desk.x, desk.width), axis(y, desk.y, desk.height))
}

/// Whether a point is inside a rect.
pub fn contains(r: &Rect, x: i32, y: i32) -> bool {
    !r.is_empty() && x >= r.x && y >= r.y && x < r.x + r.width && y < r.y + r.height
}

/// A link `launch` may open: `https://…`, `http://…` or an `ms-settings:` page. Never `file:`, scripts or
/// other protocols (those go through the shell and its approvals).
pub fn launch_url(url: &str) -> Result<String, ActError> {
    let url = url.trim();
    let refuse = |why: &str| Err(ActError::Refused(why.into()));
    if url.is_empty() || url.len() > 2048 {
        return refuse("the link is empty or too long");
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return refuse("the link has spaces or control characters");
    }
    let Some((scheme, rest)) = url.split_once(':') else {
        return refuse("the link has no scheme (use https://…)");
    };
    match scheme.to_ascii_lowercase().as_str() {
        "https" | "http" => {
            let host = rest
                .strip_prefix("//")
                .and_then(|r| r.split(['/', '?', '#']).next())
                .unwrap_or("");
            if host.is_empty() || host.starts_with('@') {
                return refuse("the link has no host");
            }
            Ok(url.to_string())
        }
        "ms-settings" => {
            if rest.is_empty()
                || !rest
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            {
                return refuse("that isn't a Settings page (e.g. ms-settings:display)");
            }
            Ok(format!("ms-settings:{rest}"))
        }
        other => Err(ActError::Refused(format!(
            "TopBar only opens https, http and ms-settings links, not {other}:"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords() {
        let c = parse_chord("ctrl+shift+s").unwrap();
        assert_eq!(c.label(), "Ctrl+Shift+S");
        assert_eq!(c.key.vk, 0x53);
        assert_eq!(
            parse_chord("Shift + Ctrl + s").unwrap().label(),
            "Ctrl+Shift+S"
        );
        assert_eq!(parse_chord("Alt+F4").unwrap().key.vk, 0x73);
        assert_eq!(
            chord_note(&parse_chord("alt+f4").unwrap()),
            Some("closes the window")
        );
        assert_eq!(parse_chord("Enter").unwrap().mods, vec![]);
        assert!(parse_chord("Ctrl+End").unwrap().key.extended);
        assert_eq!(parse_chord("Ctrl++").unwrap().label(), "Ctrl+Plus");
        assert_eq!(parse_chord("F12").unwrap().key.vk, 0x7B);
        assert_eq!(parse_chord("ctrl+5").unwrap().key.vk, 0x35);
        for bad in ["", "Ctrl+", "Hyper+S", "Ctrl+Shift", "Ctrl+Banana"] {
            assert!(parse_chord(bad).is_err(), "{bad}");
        }
        for blocked in [
            "Win+R",
            "Win",
            "Alt+Tab",
            "Alt+Shift+Tab",
            "Ctrl+Esc",
            "Ctrl+Shift+Esc",
            "Alt+Esc",
            "Ctrl+Alt+Delete",
            "meta+l",
        ] {
            let e = parse_chord(blocked).unwrap_err();
            assert!(e.contains("Windows itself"), "{blocked}: {e}");
        }
    }

    #[test]
    fn typing_units() {
        let u = type_units("hi\r\nyo\t😀").unwrap();
        assert_eq!(u[0], Unit::Char('h' as u16));
        assert_eq!(u[2], Unit::Key(ENTER));
        assert_eq!(u[5], Unit::Key(TAB));
        assert_eq!(u.len(), 8, "the emoji is a surrogate pair");
        assert!(type_units("").is_err());
        assert!(type_units(&"x".repeat(MAX_TYPED + 1)).is_err());
        assert!(type_units(&"x".repeat(MAX_TYPED)).is_ok());
        assert!(check_text(&"x".repeat(MAX_TEXT), true).is_ok());
        assert!(check_text(&"x".repeat(MAX_TEXT + 1), true).is_err());
        assert!(type_units("a\u{7}b").is_err());
        assert!(check_text("", true).is_ok());
    }

    #[test]
    fn real_keys_for_characters() {
        // VkKeyScanW on a US layout: 'a' = 0x0041, 'A' = 0x0141, '!' = 0x0131, '.' = 0x00BE.
        assert_eq!(key_for(0x0041, false), Some((0x41, false)));
        assert_eq!(key_for(0x0141, false), Some((0x41, true)));
        assert_eq!(
            key_for(0x0041, true),
            Some((0x41, true)),
            "Caps Lock: Shift for lowercase"
        );
        assert_eq!(key_for(0x0141, true), Some((0x41, false)));
        assert_eq!(
            key_for(0x0131, true),
            Some((0x31, true)),
            "Caps Lock doesn't touch digits"
        );
        assert_eq!(key_for(0x00BE, false), Some((0xBE, false)));
        assert_eq!(key_for(-1, false), None);
        assert_eq!(
            key_for(0x0645, false),
            None,
            "AltGr (Ctrl+Alt): typed as a packet"
        );
    }

    #[test]
    fn secrets() {
        for s in [
            "my key is sk-ant-api03-abcdefghijklmnopqrstuv",
            "ghp_0123456789abcdefghijABCDEFGHIJ",
            "github_pat_11ABCDEFG0123456789_abcdef",
            "AKIAIOSFODNN7EXAMPLE",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc",
            "password: Xk9vQ2mZ7rT4wLp8NcB3yH6s",
            "xoxb-123456789012-abcdef",
        ] {
            assert!(looks_secret(s), "{s}");
        }
        for s in [
            "Hello, world! This is a normal sentence.",
            "https://github.com/temimarvin/wpill/pull/123?tab=files",
            "C:\\Users\\DELL 7300\\Documents\\GitHub\\wpill\\src",
            "0f033ab5e0d4c1b2a3f4e5d6c7b8a9f0e1d2c3b4",
            "supercalifragilisticexpialidocious",
            "skip-the-line",
            "The quick brown fox jumps over the lazy dog 1234567890",
        ] {
            assert!(!looks_secret(s), "{s}");
        }
    }

    #[test]
    fn absolute_coordinates_round_trip() {
        let back = |n: i32, origin: i32, size: i32| {
            origin + (i64::from(n) * i64::from(size) / 65536) as i32
        };
        for desk in [
            Rect::from_ltrb(0, 0, 1920, 1080),
            Rect::from_ltrb(-1920, -200, 2560, 1440),
            Rect::from_ltrb(0, 0, 3840, 2160),
        ] {
            for (x, y) in [
                (desk.x, desk.y),
                (desk.x + desk.width - 1, desk.y + desk.height - 1),
                (desk.x + 812, desk.y + 40),
                (0, 0),
            ] {
                let (nx, ny) = to_absolute(x, y, desk);
                assert!((0..=65535).contains(&nx) && (0..=65535).contains(&ny));
                assert_eq!(
                    back(nx, desk.x, desk.width),
                    x.clamp(desk.x, desk.x + desk.width - 1)
                );
                assert_eq!(
                    back(ny, desk.y, desk.height),
                    y.clamp(desk.y, desk.y + desk.height - 1)
                );
            }
        }
        let r = Rect::from_ltrb(10, 10, 20, 20);
        assert!(contains(&r, 10, 19) && !contains(&r, 20, 15));
    }

    #[test]
    fn screenshot_views() {
        let v = View {
            origin: Rect::from_ltrb(100, 50, 2020, 1130),
            scale: 1920.0 / 1568.0,
            frame: Rect::default(),
        };
        assert_eq!(v.screen_point(0, 0), (100, 50));
        assert_eq!(v.screen_point(1568, 882), (2020, 1130));
    }

    #[test]
    fn links() {
        assert_eq!(
            launch_url(" https://github.com ").unwrap(),
            "https://github.com"
        );
        assert!(launch_url("http://localhost:5173/x?y#z").is_ok());
        assert_eq!(
            launch_url("MS-SETTINGS:display").unwrap(),
            "ms-settings:display"
        );
        for bad in [
            "file:///C:/Windows/System32/calc.exe",
            "C:\\x.bat",
            "javascript:alert(1)",
            "https://",
            "https:///x",
            "https://@evil",
            "https://a b",
            "ms-settings:../x",
            "ms-settings:",
            "vscode://x",
            "",
        ] {
            assert!(launch_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn verbs_and_errors() {
        assert_eq!(Verb::parse(None), Ok(Verb::Default));
        assert_eq!(Verb::parse(Some("toggle")), Ok(Verb::Toggle));
        assert!(Verb::parse(Some("smash")).is_err());
        assert_eq!(ActError::Busy.code(), "aborted");
        assert!(ActError::Aborted { done: 2, total: 5 }
            .message()
            .contains("after 2 of 5 steps"));
        assert_eq!(ActError::from(LookError::Elevated), ActError::Elevated);
    }
}

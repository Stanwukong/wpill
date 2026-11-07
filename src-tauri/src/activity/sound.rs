//! The optional "needs you" sound (Phase 2.8f, D41): off unless `activities.sound` is true in `settings.json`.
//!
//! Played when the island first shows an activity that waits on you: a session's "needs input", an approval, a
//! question or a plan. At most once per activity id (an in-place update, like a question's next step, keeps its id),
//! and never during fullscreen: activities wait while a fullscreen app runs, and the sound plays when they show.

use std::collections::VecDeque;

use windows::core::w;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT, SND_SYSTEM};

use crate::ipc::Activity;

/// Activity ids already chimed, newest last.
const REMEMBERED: usize = 64;

#[derive(Debug, Default)]
pub struct Chime {
    played: VecDeque<u32>,
}

impl Chime {
    /// Whether activity `id` should sound now; remembers it if so. Pure, so it's testable without a speaker.
    pub fn wants(&mut self, id: u32, activity: &Activity) -> bool {
        if !needs_you(activity) || self.played.contains(&id) {
            return false;
        }
        if self.played.len() >= REMEMBERED {
            self.played.pop_front();
        }
        self.played.push_back(id);
        true
    }
}

/// An activity that waits on you.
fn needs_you(activity: &Activity) -> bool {
    matches!(
        activity,
        Activity::SessionNeedsInput(_)
            | Activity::SessionApproval(_)
            | Activity::AgentApproval(_)
            | Activity::TimerDone(_)
    )
}

/// Windows' own notification sound, quietly and without blocking: the system-sounds volume, no beep if the user
/// turned it off (`SND_NODEFAULT`).
pub fn play() {
    // SAFETY: a static, NUL-terminated alias name; SND_ASYNC returns at once and Windows owns the playback.
    let played = unsafe {
        PlaySoundW(
            w!("Notification.Default"),
            None,
            SND_ALIAS | SND_ASYNC | SND_NODEFAULT | SND_SYSTEM,
        )
    };
    if !played.as_bool() {
        log::debug!("the notification sound didn't play");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{CapsLockActivity, SessionNeedsInputActivity};

    #[test]
    fn once_per_activity_and_only_when_you_are_needed() {
        let mut chime = Chime::default();
        let needs = Activity::SessionNeedsInput(SessionNeedsInputActivity {
            session_id: 1,
            agent: crate::ipc::AgentKind::Claude,
            project: "wpill".into(),
        });
        assert!(chime.wants(7, &needs));
        assert!(!chime.wants(7, &needs), "an in-place update or a reshow");
        assert!(chime.wants(8, &needs), "a new request");
        let caps = Activity::CapsLock(CapsLockActivity { on: true });
        assert!(!chime.wants(9, &caps));
        for id in 100..100 + REMEMBERED as u32 {
            chime.wants(id, &needs);
        }
        assert_eq!(chime.played.len(), REMEMBERED, "bounded");
    }
}

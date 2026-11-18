//! A short message for the status bar.

use std::time::{Duration, Instant};

/// How a notice should be presented.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    /// Something happened that the user may want to know.
    Info,
    /// Something failed.
    Error,
}

/// A message shown in the status bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// How to present it.
    pub kind: NoticeKind,
    /// What to say.
    pub text: String,
}

impl Notice {
    /// An informational notice.
    pub fn info(text: impl Into<String>) -> Notice {
        Notice {
            kind: NoticeKind::Info,
            text: text.into(),
        }
    }

    /// A save found `path` changed on disk; `reloading` says whether the folder watcher will
    /// load the new text.
    pub fn changed_on_disk(path: &str, reloading: bool) -> Notice {
        let then = if reloading {
            "reloading"
        } else {
            "reopen the vault to load it"
        };
        Notice::info(format!("{path} changed on disk; {then}"))
    }

    /// An error notice.
    pub fn error(text: impl Into<String>) -> Notice {
        Notice {
            kind: NoticeKind::Error,
            text: text.into(),
        }
    }
}

/// How long an informational notice stays in the status strip.
pub const INFO_NOTICE_TTL: Duration = Duration::from_secs(6);

/// The notice the status strip shows: the latest one, informational ones for a while.
#[derive(Debug, Default)]
pub struct NoticeBoard {
    latest: Option<(Notice, Instant)>,
}

impl NoticeBoard {
    /// Replaces the shown notice with `notice`, posted at `now`.
    pub fn post(&mut self, notice: Notice, now: Instant) {
        self.latest = Some((notice, now));
    }

    /// The notice to show at `now`; an expired informational one is dropped.
    pub fn current(&mut self, now: Instant) -> Option<&Notice> {
        if self.time_left(now) == Some(Duration::ZERO) {
            self.latest = None;
        }
        self.latest.as_ref().map(|(notice, _)| notice)
    }

    /// Time until the shown informational notice expires; `None` for errors and when empty.
    pub fn time_left(&self, now: Instant) -> Option<Duration> {
        let (notice, posted) = self.latest.as_ref()?;
        if notice.kind != NoticeKind::Info {
            return None;
        }
        Some(INFO_NOTICE_TTL.saturating_sub(now.saturating_duration_since(*posted)))
    }

    /// Removes the shown notice.
    pub fn dismiss(&mut self) {
        self.latest = None;
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn should_expire_info_but_keep_error_until_dismissed() {
        let start = Instant::now();
        let mut board = NoticeBoard::default();
        board.post(Notice::info("saved"), start);
        assert_eq!(
            board.current(start + Duration::from_secs(5)).unwrap().text,
            "saved"
        );
        assert!(board.current(start + INFO_NOTICE_TTL).is_none());
        board.post(Notice::error("failed"), start);
        let much_later = start + Duration::from_secs(3600);
        assert_eq!(board.current(much_later).unwrap().text, "failed");
        board.dismiss();
        assert!(board.current(much_later).is_none());
    }

    #[test]
    fn should_report_time_left_for_info_notice_only() {
        let start = Instant::now();
        let mut board = NoticeBoard::default();
        board.post(Notice::info("saved"), start);
        assert_eq!(
            board.time_left(start + Duration::from_secs(2)),
            Some(Duration::from_secs(4))
        );
        board.post(Notice::error("failed"), start);
        assert_eq!(board.time_left(start), None);
    }

    #[test]
    fn should_say_what_follows_when_a_note_changed_on_disk() {
        assert_eq!(
            Notice::changed_on_disk("a.md", true).text,
            "a.md changed on disk; reloading"
        );
        assert_eq!(
            Notice::changed_on_disk("a.md", false).text,
            "a.md changed on disk; reopen the vault to load it"
        );
    }

    #[test]
    fn should_build_notices_of_each_kind() {
        assert_eq!(Notice::info("a").kind, NoticeKind::Info);
        let error = Notice::error(String::from("b"));
        assert_eq!((error.kind, error.text.as_str()), (NoticeKind::Error, "b"));
    }
}

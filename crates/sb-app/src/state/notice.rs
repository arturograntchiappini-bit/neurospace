//! A short message for the status bar.

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

    /// An error notice.
    pub fn error(text: impl Into<String>) -> Notice {
        Notice {
            kind: NoticeKind::Error,
            text: text.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_build_notices_of_each_kind() {
        assert_eq!(Notice::info("a").kind, NoticeKind::Info);
        let error = Notice::error(String::from("b"));
        assert_eq!((error.kind, error.text.as_str()), (NoticeKind::Error, "b"));
    }
}

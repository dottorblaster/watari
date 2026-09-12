use std::fmt;

use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text(Vec<u8>),
    Cleared,
    NonText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    NoText,
    Unchanged,
    ClearsDisabled,
    AlreadyCleared,
}

impl fmt::Display for SkipReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            SkipReason::NoText => "CLIPBOARD has no textual target",
            SkipReason::Unchanged => "CLIPBOARD is unchanged",
            SkipReason::ClearsDisabled => "CLIPBOARD was cleared but MIRROR_CLEARS is off",
            SkipReason::AlreadyCleared => "CLIPBOARD is already cleared",
        };
        formatter.write_str(message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    SetText(Vec<u8>),
    Clear,
    Skip(SkipReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    MirroredText(usize),
    MirroredClear,
    Skipped(SkipReason),
}

#[derive(Debug, Default)]
pub struct MirrorState {
    last: Option<Vec<u8>>,
}

impl MirrorState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last(&self) -> Option<&[u8]> {
        self.last.as_deref()
    }

    pub fn decide(&self, content: Content, mirror_clears: bool) -> Decision {
        match content {
            Content::NonText => Decision::Skip(SkipReason::NoText),
            Content::Cleared => self.decide_clear(mirror_clears),
            Content::Text(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned().into_bytes();
                if text.is_empty() {
                    self.decide_clear(mirror_clears)
                } else if self.last() == Some(text.as_slice()) {
                    Decision::Skip(SkipReason::Unchanged)
                } else {
                    Decision::SetText(text)
                }
            }
        }
    }

    pub fn record(&mut self, decision: &Decision) {
        match decision {
            Decision::SetText(text) => self.last = Some(text.clone()),
            Decision::Clear => self.last = Some(Vec::new()),
            Decision::Skip(_) => {}
        }
    }

    fn decide_clear(&self, mirror_clears: bool) -> Decision {
        if !mirror_clears {
            Decision::Skip(SkipReason::ClearsDisabled)
        } else if matches!(self.last(), Some([])) {
            Decision::Skip(SkipReason::AlreadyCleared)
        } else {
            Decision::Clear
        }
    }
}

pub trait Source {
    fn wait_for_change(&mut self) -> Result<()>;
    fn current(&mut self) -> Result<Content>;
    fn has_pending(&mut self) -> bool;
}

pub trait Sink {
    fn set_text(&mut self, bytes: &[u8]) -> Result<()>;
    fn clear(&mut self) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Content {
        Content::Text(value.as_bytes().to_vec())
    }

    #[test]
    fn new_text_is_selected_and_recorded() {
        let mut state = MirrorState::new();
        let decision = state.decide(text("secret"), true);
        assert_eq!(decision, Decision::SetText(b"secret".to_vec()));
        state.record(&decision);
        assert_eq!(state.last(), Some(b"secret".as_slice()));
    }

    #[test]
    fn identical_text_is_skipped() {
        let mut state = MirrorState::new();
        state.record(&Decision::SetText(b"same".to_vec()));
        assert_eq!(
            state.decide(text("same"), true),
            Decision::Skip(SkipReason::Unchanged)
        );
    }

    #[test]
    fn invalid_utf8_is_made_lossy() {
        let state = MirrorState::new();
        let decision = state.decide(Content::Text(vec![0xff, 0xfe, b'a']), true);
        assert_eq!(
            decision,
            Decision::SetText("\u{FFFD}\u{FFFD}a".as_bytes().to_vec())
        );
    }

    #[test]
    fn non_text_is_never_mirrored() {
        let state = MirrorState::new();
        assert_eq!(
            state.decide(Content::NonText, true),
            Decision::Skip(SkipReason::NoText)
        );
    }

    #[test]
    fn cleared_empty_and_owner_gone_all_map_to_clear() {
        let state = MirrorState::new();
        assert_eq!(state.decide(Content::Cleared, true), Decision::Clear);
        assert_eq!(state.decide(text(""), true), Decision::Clear);
    }

    #[test]
    fn clears_can_be_disabled() {
        let state = MirrorState::new();
        assert_eq!(
            state.decide(Content::Cleared, false),
            Decision::Skip(SkipReason::ClearsDisabled)
        );
        assert_eq!(
            state.decide(text(""), false),
            Decision::Skip(SkipReason::ClearsDisabled)
        );
    }

    #[test]
    fn repeated_clears_are_deduplicated() {
        let mut state = MirrorState::new();
        let decision = state.decide(Content::Cleared, true);
        assert_eq!(decision, Decision::Clear);
        state.record(&decision);
        assert_eq!(
            state.decide(Content::Cleared, true),
            Decision::Skip(SkipReason::AlreadyCleared)
        );
    }

    #[test]
    fn text_replaces_a_recorded_clear() {
        let mut state = MirrorState::new();
        state.record(&Decision::Clear);
        assert_eq!(
            state.decide(text("after"), true),
            Decision::SetText(b"after".to_vec())
        );
    }

    #[test]
    fn skipped_decisions_do_not_change_state() {
        let mut state = MirrorState::new();
        state.record(&Decision::SetText(b"keep".to_vec()));
        state.record(&Decision::Skip(SkipReason::Unchanged));
        assert_eq!(state.last(), Some(b"keep".as_slice()));
    }
}

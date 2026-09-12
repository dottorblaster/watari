use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use log::{debug, info, warn};

use crate::config::Config;
use crate::mirror::{Decision, MirrorState, Outcome, Sink, Source};
use crate::wayland::WaylandSink;
use crate::x11::X11Source;

pub const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
pub const MAX_BACKOFF: Duration = Duration::from_secs(30);

pub fn mirror_once<S: Source, K: Sink>(
    source: &mut S,
    sink: &mut K,
    state: &mut MirrorState,
    mirror_clears: bool,
) -> Result<Outcome> {
    let content = source.current()?;
    let decision = state.decide(content, mirror_clears);

    let outcome = match &decision {
        Decision::SetText(text) => {
            sink.set_text(text).context("set the Wayland clipboard")?;
            Outcome::MirroredText(text.len())
        }
        Decision::Clear => {
            sink.clear().context("clear the Wayland clipboard")?;
            Outcome::MirroredClear
        }
        Decision::Skip(reason) => Outcome::Skipped(*reason),
    };

    state.record(&decision);
    Ok(outcome)
}

pub fn watch<S: Source, K: Sink>(source: &mut S, sink: &mut K, mirror_clears: bool) -> Result<()> {
    let mut state = MirrorState::new();
    loop {
        source.wait_for_change()?;
        loop {
            let outcome = mirror_once(source, sink, &mut state, mirror_clears)?;
            log_outcome(&outcome);
            if !source.has_pending() {
                break;
            }
        }
    }
}

pub fn run(config: &Config) -> Result<()> {
    info!(
        "watari: mirroring X CLIPBOARD to the Wayland clipboard (display={}, MIRROR_CLEARS={})",
        config.display, config.mirror_clears
    );

    let mut backoff = INITIAL_BACKOFF;

    loop {
        match X11Source::connect(Some(&config.display)) {
            Ok(mut source) => {
                backoff = INITIAL_BACKOFF;
                info!("connected to the X server; watching the CLIPBOARD selection");
                let mut sink = WaylandSink::system();
                if let Err(err) = watch(&mut source, &mut sink, config.mirror_clears) {
                    warn!("X session ended: {err:#}");
                }
            }
            Err(err) => warn!("X connection unavailable: {err:#}"),
        }

        warn!("reconnecting in {backoff:?}");
        thread::sleep(backoff);
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

fn log_outcome(outcome: &Outcome) {
    match outcome {
        Outcome::MirroredText(len) => {
            info!("mirrored X CLIPBOARD -> Wayland ({len} bytes)")
        }
        Outcome::MirroredClear => info!("mirrored X CLIPBOARD clear -> Wayland"),
        Outcome::Skipped(reason) => debug!("skipped: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::mirror::{Content, SkipReason};

    #[derive(Debug, Default, PartialEq, Eq)]
    struct FakeSink {
        texts: Vec<Vec<u8>>,
        clears: usize,
    }

    impl Sink for FakeSink {
        fn set_text(&mut self, bytes: &[u8]) -> Result<()> {
            self.texts.push(bytes.to_vec());
            Ok(())
        }

        fn clear(&mut self) -> Result<()> {
            self.clears += 1;
            Ok(())
        }
    }

    struct FakeSource {
        contents: VecDeque<Content>,
        pending: VecDeque<bool>,
    }

    impl FakeSource {
        fn new(contents: Vec<Content>, pending: Vec<bool>) -> Self {
            Self {
                contents: contents.into(),
                pending: pending.into(),
            }
        }
    }

    impl Source for FakeSource {
        fn wait_for_change(&mut self) -> Result<()> {
            if self.contents.is_empty() {
                anyhow::bail!("simulated disconnect");
            }
            Ok(())
        }

        fn current(&mut self) -> Result<Content> {
            self.contents
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("no content"))
        }

        fn has_pending(&mut self) -> bool {
            self.pending.pop_front().unwrap_or(false)
        }
    }

    #[test]
    fn mirror_once_publishes_text_and_remembers_it() {
        let mut source = FakeSource::new(vec![Content::Text(b"secret".to_vec())], vec![]);
        let mut sink = FakeSink::default();
        let mut state = MirrorState::new();

        let outcome = mirror_once(&mut source, &mut sink, &mut state, true).unwrap();

        assert_eq!(outcome, Outcome::MirroredText(6));
        assert_eq!(sink.texts, vec![b"secret".to_vec()]);
        assert_eq!(state.last(), Some(b"secret".as_slice()));
    }

    #[test]
    fn mirror_once_clears_only_when_enabled() {
        let mut source = FakeSource::new(vec![Content::Cleared, Content::Cleared], vec![]);
        let mut sink = FakeSink::default();
        let mut state = MirrorState::new();

        let cleared = mirror_once(&mut source, &mut sink, &mut state, true).unwrap();
        assert_eq!(cleared, Outcome::MirroredClear);

        let disabled = mirror_once(&mut source, &mut sink, &mut state, false).unwrap();
        assert_eq!(disabled, Outcome::Skipped(SkipReason::ClearsDisabled));
        assert_eq!(sink.clears, 1);
    }

    #[test]
    fn mirror_once_ignores_non_text() {
        let mut source = FakeSource::new(vec![Content::NonText], vec![]);
        let mut sink = FakeSink::default();
        let mut state = MirrorState::new();

        let outcome = mirror_once(&mut source, &mut sink, &mut state, true).unwrap();

        assert_eq!(outcome, Outcome::Skipped(SkipReason::NoText));
        assert!(sink.texts.is_empty());
        assert_eq!(sink.clears, 0);
    }

    #[test]
    fn mirror_once_deduplicates_identical_text() {
        let mut source = FakeSource::new(
            vec![
                Content::Text(b"same".to_vec()),
                Content::Text(b"same".to_vec()),
            ],
            vec![],
        );
        let mut sink = FakeSink::default();
        let mut state = MirrorState::new();

        mirror_once(&mut source, &mut sink, &mut state, true).unwrap();
        let second = mirror_once(&mut source, &mut sink, &mut state, true).unwrap();

        assert_eq!(second, Outcome::Skipped(SkipReason::Unchanged));
        assert_eq!(sink.texts, vec![b"same".to_vec()]);
    }

    #[test]
    fn source_errors_propagate() {
        struct FailingSource;
        impl Source for FailingSource {
            fn wait_for_change(&mut self) -> Result<()> {
                anyhow::bail!("lost")
            }
            fn current(&mut self) -> Result<Content> {
                anyhow::bail!("lost")
            }
            fn has_pending(&mut self) -> bool {
                false
            }
        }

        let mut source = FailingSource;
        let mut sink = FakeSink::default();
        let mut state = MirrorState::new();
        assert!(mirror_once(&mut source, &mut sink, &mut state, true).is_err());
    }

    #[test]
    fn sink_errors_propagate() {
        struct FailingSink;
        impl Sink for FailingSink {
            fn set_text(&mut self, _bytes: &[u8]) -> Result<()> {
                anyhow::bail!("no compositor")
            }
            fn clear(&mut self) -> Result<()> {
                anyhow::bail!("no compositor")
            }
        }

        let mut source = FakeSource::new(vec![Content::Text(b"x".to_vec())], vec![]);
        let mut sink = FailingSink;
        let mut state = MirrorState::new();
        let error = mirror_once(&mut source, &mut sink, &mut state, true).unwrap_err();
        assert!(error.to_string().contains("set the Wayland clipboard"));
    }

    #[test]
    fn watch_mirrors_until_the_source_disconnects() {
        let mut source = FakeSource::new(
            vec![Content::Text(b"one".to_vec()), Content::Cleared],
            vec![false, false],
        );
        let mut sink = FakeSink::default();

        let error = watch(&mut source, &mut sink, true).unwrap_err();
        assert!(error.to_string().contains("simulated disconnect"));
        assert_eq!(sink.texts, vec![b"one".to_vec()]);
        assert_eq!(sink.clears, 1);
    }

    #[test]
    fn watch_reprocesses_when_changes_arrive_mid_transfer() {
        let mut source = FakeSource::new(
            vec![
                Content::Text(b"one".to_vec()),
                Content::Text(b"two".to_vec()),
            ],
            vec![true, false],
        );
        let mut sink = FakeSink::default();

        watch(&mut source, &mut sink, true).unwrap_err();

        assert_eq!(sink.texts, vec![b"one".to_vec(), b"two".to_vec()]);
    }
}

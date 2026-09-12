use anyhow::{Context, Result};
use wl_clipboard_rs::copy::{clear, ClipboardType, MimeType, Options, Seat, Source as WlSource};

use crate::mirror::Sink;

pub trait Backend {
    fn set_text(&mut self, bytes: &[u8]) -> Result<()>;
    fn clear(&mut self) -> Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WaylandBackend;

impl Backend for WaylandBackend {
    fn set_text(&mut self, bytes: &[u8]) -> Result<()> {
        // Serving model: a Wayland selection is not a value the compositor stores, it is a
        // live data source. Whoever sets the selection must keep serving it until somebody
        // else takes the selection over. wl-clipboard-rs::copy with the default
        // (non-foreground) options spawns a background thread that owns its own Wayland
        // connection and answers paste requests; when another client becomes the selection
        // owner the compositor delivers the `cancelled` event, the old source is destroyed
        // and that thread exits on its own. So repeated calls do not pile anything up: the
        // previous server tears itself down as soon as the new selection replaces it, and
        // there is deliberately no PID management here.
        Options::new()
            .copy(WlSource::Bytes(bytes.to_vec().into()), MimeType::Text)
            .context("set the Wayland clipboard")
    }

    fn clear(&mut self) -> Result<()> {
        clear(ClipboardType::Regular, Seat::All).context("clear the Wayland clipboard")
    }
}

pub struct WaylandSink<B: Backend = WaylandBackend> {
    backend: B,
}

impl<B: Backend> WaylandSink<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn into_backend(self) -> B {
        self.backend
    }
}

impl WaylandSink<WaylandBackend> {
    pub fn system() -> Self {
        Self::new(WaylandBackend)
    }
}

impl<B: Backend> Sink for WaylandSink<B> {
    fn set_text(&mut self, bytes: &[u8]) -> Result<()> {
        self.backend.set_text(bytes)
    }

    fn clear(&mut self) -> Result<()> {
        self.backend.clear()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, PartialEq, Eq)]
    struct RecordingBackend {
        texts: Vec<Vec<u8>>,
        clears: usize,
    }

    impl Backend for RecordingBackend {
        fn set_text(&mut self, bytes: &[u8]) -> Result<()> {
            self.texts.push(bytes.to_vec());
            Ok(())
        }

        fn clear(&mut self) -> Result<()> {
            self.clears += 1;
            Ok(())
        }
    }

    #[test]
    fn sink_forwards_text_and_clear_to_the_backend() {
        let mut sink = WaylandSink::new(RecordingBackend::default());

        Sink::set_text(&mut sink, b"hello").unwrap();
        Sink::clear(&mut sink).unwrap();
        Sink::set_text(&mut sink, b"world").unwrap();

        assert_eq!(
            sink.into_backend(),
            RecordingBackend {
                texts: vec![b"hello".to_vec(), b"world".to_vec()],
                clears: 1,
            }
        );
    }

    #[test]
    fn errors_from_the_backend_are_propagated() {
        #[derive(Default)]
        struct FailingBackend;

        impl Backend for FailingBackend {
            fn set_text(&mut self, _bytes: &[u8]) -> Result<()> {
                anyhow::bail!("no compositor")
            }

            fn clear(&mut self) -> Result<()> {
                anyhow::bail!("no compositor")
            }
        }

        let mut sink = WaylandSink::new(FailingBackend);
        assert!(Sink::set_text(&mut sink, b"x").is_err());
        assert!(Sink::clear(&mut sink).is_err());
    }

    #[test]
    #[ignore = "requires a running Wayland compositor; run with --ignored"]
    fn real_backend_sets_and_clears() {
        let mut backend = WaylandBackend;
        backend.set_text(b"watari test").unwrap();
        backend.clear().unwrap();
    }
}

use anyhow::{Context, Result};
use log::debug;
use x11rb::connection::Connection;
use x11rb::protocol::xfixes::{ConnectionExt as _, SelectionEvent, SelectionEventMask};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, GetPropertyReply, Property,
    Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::{atom_manager, COPY_FROM_PARENT, CURRENT_TIME, NONE};

use crate::mirror::{Content, Source};

atom_manager! {
    pub Atoms:
    AtomsCookie {
        CLIPBOARD,
        TARGETS,
        UTF8_STRING,
        TEXT_PLAIN_UTF8: b"text/plain;charset=utf-8",
        STRING,
        INCR,
        WATARI,
    }
}

pub struct X11Source {
    conn: RustConnection,
    atoms: Atoms,
    requestor: Window,
    prop: Atom,
    pending: bool,
}

impl X11Source {
    pub fn connect(display: Option<&str>) -> Result<Self> {
        let (conn, screen_num) = x11rb::connect(display).context("connect to the X server")?;
        let root = conn.setup().roots[screen_num].root;

        let atoms = Atoms::new(&conn)
            .context("request clipboard atoms")?
            .reply()
            .context("intern clipboard atoms")?;

        conn.xfixes_query_version(5, 0)
            .context("query the XFIXES version")?
            .reply()
            .context("read the XFIXES version")?;

        // SelectSelectionInput is what clipnotify does under the hood: rather than polling
        // the selection owner on a timer, we ask the server to push a single event whenever
        // the CLIPBOARD owner changes. We listen on the root window because any client may
        // own the selection, and only for SET_SELECTION_OWNER so a released selection (owner
        // NONE) shows up as an owner change too.
        conn.xfixes_select_selection_input(
            root,
            atoms.CLIPBOARD,
            SelectionEventMask::SET_SELECTION_OWNER,
        )
        .context("select CLIPBOARD owner-change events")?;

        let requestor = conn
            .generate_id()
            .context("allocate a requestor window id")?;
        conn.create_window(
            0,
            requestor,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            COPY_FROM_PARENT,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .context("create the hidden requestor window")?;
        conn.flush().context("flush the X setup requests")?;

        let prop = atoms.WATARI;
        Ok(Self {
            conn,
            atoms,
            requestor,
            prop,
            pending: false,
        })
    }

    fn fetch_text(&mut self) -> Result<Option<Vec<u8>>> {
        let Some(_) = self.request_conversion(self.atoms.TARGETS)? else {
            return Ok(None);
        };
        let targets = self.read_property(true, self.prop)?;
        let available: Vec<Atom> = targets.value32().map(|it| it.collect()).unwrap_or_default();
        debug!("CLIPBOARD offers {} target(s)", available.len());

        let preference = [
            self.atoms.UTF8_STRING,
            self.atoms.TEXT_PLAIN_UTF8,
            self.atoms.STRING,
        ];
        let Some(target) = choose_target(&available, &preference) else {
            return Ok(None);
        };
        debug!("selected text target atom {target}");

        let Some(_) = self.request_conversion(target)? else {
            return Ok(None);
        };
        let reply = self.read_property(false, self.prop)?;
        if reply.type_ == self.atoms.INCR {
            if let Some(size) = reply.value32().and_then(|mut it| it.next()) {
                debug!("INCR transfer: owner estimates {size} bytes");
            }
            return self.read_incr(target).map(Some);
        }
        debug!("direct transfer: {} bytes", reply.value.len());
        Ok(Some(reply.value))
    }

    fn request_conversion(&mut self, target: Atom) -> Result<Option<Atom>> {
        let requestor = self.requestor;
        let clipboard = self.atoms.CLIPBOARD;
        let property = self.prop;

        // The ConvertSelection/SelectionNotify dance is asynchronous: we name a target and a
        // property on our requestor window, then block until the server relays the owner's
        // SelectionNotify. A NONE property means the owner declined the target, which is a
        // normal "not available" answer rather than an error.
        self.conn
            .convert_selection(requestor, clipboard, target, property, CURRENT_TIME)
            .context("request a selection conversion")?;
        self.conn
            .flush()
            .context("flush the selection conversion")?;

        self.wait_for_event(move |event| match event {
            Event::SelectionNotify(notify)
                if notify.requestor == requestor
                    && notify.selection == clipboard
                    && notify.target == target =>
            {
                Some((notify.property != NONE).then_some(notify.property))
            }
            _ => None,
        })
    }

    fn read_property(&mut self, delete: bool, property: Atom) -> Result<GetPropertyReply> {
        let requestor = self.requestor;
        let bytes_after = self
            .conn
            .get_property(false, requestor, property, AtomEnum::ANY, 0, 0)
            .context("probe the property size")?
            .reply()
            .context("read the property size")?
            .bytes_after;

        self.conn
            .get_property(
                delete,
                requestor,
                property,
                AtomEnum::ANY,
                0,
                bytes_after.div_ceil(4),
            )
            .context("read the property")?
            .reply()
            .context("read the property reply")
    }

    fn read_incr(&mut self, target: Atom) -> Result<Vec<u8>> {
        let requestor = self.requestor;
        let property = self.prop;

        // INCR protocol. The owner refused to drop the payload straight into the property
        // (typically because it is larger than the maximum request size) and instead wrote a
        // 32-bit size marker whose type is INCR. The transfer is requestor-driven from here:
        // deleting the marker tells the owner "send the first chunk", and every subsequent
        // delete-read tells it "send the next chunk". Each chunk lands in the same property
        // and raises a PropertyNotify(NewValue) on our window, so we read+delete in a loop
        // until the owner writes a zero-length chunk, which is its end-of-stream marker.
        // Deleting between reads is what keeps the pipeline moving; reading without deleting
        // would stall the owner forever. The `type_ != target` guard filters out the stale
        // PropertyNotify for the marker itself and any not-yet-payload write, which is the
        // race that trips up naive implementations.
        self.conn
            .delete_property(requestor, property)
            .context("delete the INCR marker")?;
        self.conn.flush().context("flush the INCR delete")?;

        let mut data = Vec::new();
        loop {
            self.wait_for_event(move |event| match event {
                Event::PropertyNotify(notify)
                    if notify.window == requestor
                        && notify.atom == property
                        && notify.state == Property::NEW_VALUE =>
                {
                    Some(())
                }
                _ => None,
            })?;

            let chunk = self.read_property(true, property)?;
            if chunk.type_ != target {
                continue;
            }
            if chunk.value.is_empty() {
                debug!("INCR transfer complete: {} bytes total", data.len());
                return Ok(data);
            }
            debug!("INCR chunk: {} bytes", chunk.value.len());
            data.extend_from_slice(&chunk.value);
        }
    }

    fn wait_for_event<F, T>(&mut self, mut extract: F) -> Result<T>
    where
        F: FnMut(&Event) -> Option<T>,
    {
        loop {
            let event = self.conn.wait_for_event().context("wait for an X event")?;

            if let Event::XfixesSelectionNotify(notify) = &event {
                if is_owner_change(notify.subtype, notify.selection, self.atoms.CLIPBOARD) {
                    self.pending = true;
                }
            }
            if let Event::Error(error) = &event {
                debug!("X protocol error: {error:?}");
            }

            if let Some(value) = extract(&event) {
                return Ok(value);
            }
        }
    }
}

impl Source for X11Source {
    fn wait_for_change(&mut self) -> Result<()> {
        self.pending = false;
        loop {
            let event = self.conn.wait_for_event().context("wait for an X event")?;
            if let Event::XfixesSelectionNotify(notify) = &event {
                if is_owner_change(notify.subtype, notify.selection, self.atoms.CLIPBOARD) {
                    return Ok(());
                }
            }
        }
    }

    fn current(&mut self) -> Result<Content> {
        let owner = self
            .conn
            .get_selection_owner(self.atoms.CLIPBOARD)
            .context("query the CLIPBOARD owner")?
            .reply()
            .context("read the CLIPBOARD owner")?
            .owner;

        let text = if owner == NONE {
            None
        } else {
            self.fetch_text()?
        };
        Ok(classify_owner(owner, text))
    }

    fn has_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}

pub fn choose_target(available: &[Atom], preference: &[Atom]) -> Option<Atom> {
    preference
        .iter()
        .copied()
        .find(|candidate| available.contains(candidate))
}

pub fn classify_owner(owner: Window, text: Option<Vec<u8>>) -> Content {
    if owner == NONE {
        Content::Cleared
    } else {
        text.map(Content::Text).unwrap_or(Content::NonText)
    }
}

pub fn is_owner_change(subtype: SelectionEvent, selection: Atom, clipboard: Atom) -> bool {
    subtype == SelectionEvent::SET_SELECTION_OWNER && selection == clipboard
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIPBOARD: Atom = 100;
    const OTHER: Atom = 101;
    const UTF8: Atom = 10;
    const CHARSET: Atom = 11;
    const STRING: Atom = 12;

    #[test]
    fn target_preference_order() {
        let preference = [UTF8, CHARSET, STRING];
        assert_eq!(
            choose_target(&[STRING, CHARSET, UTF8], &preference),
            Some(UTF8)
        );
        assert_eq!(
            choose_target(&[STRING, CHARSET], &preference),
            Some(CHARSET)
        );
        assert_eq!(choose_target(&[STRING], &preference), Some(STRING));
        assert_eq!(choose_target(&[], &preference), None);
    }

    #[test]
    fn missing_owner_is_a_clear() {
        assert_eq!(classify_owner(NONE, None), Content::Cleared);
        assert_eq!(
            classify_owner(NONE, Some(b"ignored".to_vec())),
            Content::Cleared
        );
    }

    #[test]
    fn present_owner_with_text_is_text() {
        assert_eq!(
            classify_owner(7, Some(b"hi".to_vec())),
            Content::Text(b"hi".to_vec())
        );
    }

    #[test]
    fn present_owner_without_text_is_non_text() {
        assert_eq!(classify_owner(7, None), Content::NonText);
    }

    #[test]
    fn only_clipboard_owner_changes_count() {
        assert!(is_owner_change(
            SelectionEvent::SET_SELECTION_OWNER,
            CLIPBOARD,
            CLIPBOARD
        ));
        assert!(!is_owner_change(
            SelectionEvent::SET_SELECTION_OWNER,
            OTHER,
            CLIPBOARD
        ));
        assert!(!is_owner_change(
            SelectionEvent::SELECTION_CLIENT_CLOSE,
            CLIPBOARD,
            CLIPBOARD
        ));
    }

    #[test]
    fn connecting_to_a_missing_display_fails_with_context() {
        let error = X11Source::connect(Some(":231"))
            .err()
            .expect("connecting to a missing display should fail");
        assert!(error.to_string().contains("connect to the X server"));
    }
}

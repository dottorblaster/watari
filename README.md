# watari

X11 clipboard → Wayland clipboard. One direction, no polling, no `xsel`, no
`wl-copy`, no shell loop held together with tape.

The setup that needs this: niri (or anything else that only speaks
`ext-data-control-v1`) plus an XWayland app writing through the old
`wlr-data-control`. Bitwarden Desktop is the usual culprit, for example.
You copy a password, walk to a Wayland app, paste nothing. watari drags the text across.

It replaces `clipnotify` and a bash loop that babysits it:

```sh
while true; do
    clipnotify -s clipboard
    cur="$(xsel --clipboard --output)"
    [ "$cur" = "$last" ] && continue
    last="$cur"
    printf '%s' "$cur" | wl-copy
done
```

## Build

```sh
cargo build --release
install -Dm755 target/release/watari ~/.local/bin/watari
```

## Run at login

niri (`config.kdl`):

```kdl
spawn-at-startup "watari"
```

Hyprland (`hyprland.conf`):

```conf
exec-once = watari
```

Neither needs a `sleep` wrapper. watari waits for XWayland to appear, and if
the satellite later dies and comes back it reconnects with backoff. It does not
exit.

Need env vars? Wrap it:

```kdl
# niri
spawn-at-startup "sh" "-c" "RUST_LOG=info exec watari"
```

```conf
# Hyprland
exec-once = sh -c 'RUST_LOG=info exec watari'
```

Hyprland can also set them globally instead: `env = MIRROR_CLEARS,0` above the
`exec-once` line.

Environment:

| Variable        | Default | What it does                                                                              |
| --------------- | ------- | ----------------------------------------------------------------------------------------- |
| `DISPLAY`       | `:0`    | X server to talk to.                                                                      |
| `MIRROR_CLEARS` | `true`  | Mirror a wiped X clipboard (Bitwarden's timeout wipe). Off means the secret lingers Wayland-side. |
| `RUST_LOG`      | `info`  | Set to `debug` for transfer details.                                                      |

Flags win over env:

```sh
watari --display=:0
watari --display :0
watari --no-mirror-clears
watari --mirror-clears=false
watari --print-config      # show what it resolved, then exit
watari --help
watari --version
```

## What it actually does

- Subscribes to `SelectSelectionInput` via XFIXES, same trick as `clipnotify`.
  Events arrive; the CPU does nothing in between.
- Reads `TARGETS`, picks `UTF8_STRING` → `text/plain;charset=utf-8` → `STRING`,
  and implements the ICCCM `INCR` chunking so large selections don't come back
  truncated.
- Remembers the last value so it never writes the same thing twice.
- Hands text to `wl-clipboard-rs`, which sets the Wayland selection and keeps a
  background thread alive to serve paste requests until something else takes
  over.

Only `CLIPBOARD`. `PRIMARY` is left alone, so nothing yanks the selection out
from under your GTK apps.

Logs say how many bytes moved and never what moved. It's a password manager on
the other end, not a debug toy.

## Tests

```sh
cargo test                  # everything that doesn't need a compositor
cargo test -- --ignored     # the one test that does
```

The X and Wayland sides sit behind traits, so the loop, dedup and error paths
run against fakes. The binary itself is covered with `assert_cmd`.

## License

Apache-2.0. See [LICENSE](LICENSE).

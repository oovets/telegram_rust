# telegram_rust

[![Rust](https://img.shields.io/badge/rust-stable-DEA584.svg)](https://www.rust-lang.org/)
[![Docs](https://img.shields.io/badge/docs-mkdocs--material-blue.svg)](https://stevoo.net/telegram_rust/)

a fast, keyboard-first telegram client for terminal workflows. for people who live in
panes, move quickly with shortcuts, and want multi-chat focus without leaving the shell.

!](docs/screenshot.svg)

```
== why this client ==

- fast redraws and responsive input in a native tui

- multi-pane chat workflow (split, focus, close, clear)

- strong keyboard ergonomics + mouse support

- rich message formatting (replies, reactions, media labels, aliases)

- persistent layout/settings between sessions

- kitty inline image preview with zoom + next/previous navigation

- lightweight footprint: ~10 MB RAM and <1% CPU at idle
```

```
== feature highlights ==

split view      vertical/horizontal split, per-pane focus + input buffers,
                direction toggle, pane close

chat org        sidebar sections unread / active / other / muted; mute/unmute
                (muted grouped under other); unread marker + optional counts

message ux      reply mode with inline preview; search, media fetch/open, forward,
                edit, delete; sender aliases; optional reactions / emojis /
                timestamps / line numbers / compact mode

inline media    /media N opens image preview inline in kitty; esc close; +/- zoom;
                n/p or right/left next/previous image in the chat

persistence     saves pane tree/layout, focused pane, muted chats, display
                settings and aliases
```

```
== tui layout ==

+--------- Chats ---------+ +----------- Pane 1 -----------+ +---- Preview ----+
| Unread                  | | Chat Header (name / typing)  | |  Preview: img   |
| ▶ (3) Team Alpha        | |------------------------------| |                 |
| Active                  | | #120 10:42 Alice: shipping   | |  [ inline img ] |
| @bob                    | | #121 10:43 You: ok           | |                 |
| Other                   | |   ↳ Reply to Alice: looks ok | |                 |
| Project Logs            | |------------------------------| |-----------------|
| Muted                   | | Input (Alt+Enter newline)    | | Esc close +/- z |
+-------------------------+ +------------------------------+ +-----------------+
```

```
== keyboard shortcuts ==

global / nav    Ctrl+Q quit · Ctrl+R refresh · Tab/Shift+Tab cycle focus ·
                Alt+Left/Right prev/next pane · Ctrl+Left/Right resize sidebar ·
                Up/Down navigate list or input history · Enter open chat / send ·
                Alt+Enter newline · Esc cancel reply (or close inline preview)

panes           Ctrl+V split vertical · Ctrl+B split horizontal · Ctrl+K toggle
                direction · Ctrl+W close · Ctrl+L clear · PageUp/PageDown scroll

display/chat    Ctrl+E reactions · Ctrl+N notifications · Ctrl+D compact ·
                Ctrl+O emojis · Ctrl+G line numbers · Ctrl+T timestamps ·
                Ctrl+S chat list · Ctrl+M unread count · Ctrl+P mute/unmute ·
                Ctrl+Y borders

inline preview  Esc close · + / = zoom in · - zoom out · n/Right next · p/Left prev
```

```
== commands ==

/reply <N>  (/r)          reply to message #N
/search <q> (/s)          search current chat
/media <N>  (/m)          download/open media from #N (kitty: inline image preview)
/edit <N> <text> (/e)     edit message #N
/delete <N> (/d)          delete message #N
/alias <N> <name>         alias sender of message #N
/unalias <N>              remove alias for sender of #N
/filter <type|sender>     filter by media/sender/link · /filter off disables
/new @username            open dm by username
/newgroup <name>          create group
/add @username            add user to current group
/kick @username (/remove) remove user
/members                  list current group members
/forward <N> @user (/fwd) forward message
```

```bash
# install & run -- first run needs telegram api credentials from https://my.telegram.org
cd telegram_client_rs
cargo build --release
./target/release/telegram_client_rs   # or: cargo run --release
```

```text
== project structure ==

src/
  main.rs           event loop, key/mouse input handling
  app.rs            main ui + state + pane/chat behavior
  commands.rs       slash commands
  telegram.rs       telegram api integration
  split_view.rs     pane tree and rendering split logic
  formatting.rs     message formatting and wrapping
  persistence.rs    layout/settings/aliases persistence
  kitty_preview.rs  kitty graphics protocol rendering
  utils.rs          notifications, autocomplete, helpers
  widgets.rs        chat/message data structures
```

```json
// telegram_config.json (first run)
{ "api_id": 123456, "api_hash": "your_hash_here" }
```

```json
// telegram_aliases.json
{ "123456789": "Alice", "987654321": "Bob" }
```

telegram_layout.json is saved automatically with pane layout, focused pane, and muted chats.

```bash
# development
cargo build
cargo test
cargo check
cargo fmt
cargo clippy
```

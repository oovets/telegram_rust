# Telegram Client - Rust TUI

A fast, keyboard-first Telegram client built for terminal workflows.  
Designed for people who live in panes, move quickly with shortcuts, and want multi-chat focus without leaving the shell.

## Why This Client

- Fast redraws and responsive input in a native TUI
- Multi-pane chat workflow (split, focus, close, clear)
- Strong keyboard ergonomics + mouse support
- Rich message formatting (replies, reactions, media labels, aliases)
- Persistent layout/settings between sessions
- Kitty inline image preview with zoom + next/previous navigation

## Feature Highlights

- **Split view system**
  - Vertical/horizontal pane split
  - Per-pane focus and per-pane input buffers
  - Pane direction toggle and pane close
- **Chat organization**
  - Sidebar sections: `Unread`, `Active`, `Other`, `Muted`
  - Mute/unmute chats (muted chats grouped under `Other`)
  - Unread marker and optional unread counts
- **Message UX**
  - Reply mode with inline preview
  - Search, media fetch/open, forward, edit, delete
  - Sender aliases
  - Optional reactions, emojis, timestamps, line numbers, compact mode
- **Inline media preview (Kitty)**
  - `/media N` opens image preview inline in Kitty
  - `Esc` close preview
  - `+/-` zoom
  - `n/p` or `Right/Left` next/previous image in the current chat
- **Persistence**
  - Saves pane tree/layout, focused pane, muted chats
  - Saves display settings and aliases

## TUI Layout (ASCII)

```text
+-------------------- Chats ---------+ +------------------ Pane 1 ------------------  + +------- Preview --------+
| Unread                             | | Chat Header (name / username / typing)       | |   Preview: image.png   |
| ▶ (3) Team Alpha                   | |----------------------------------------------| |                        |
| Active                             | | #120 10:42 Alice: shipping now               | |      [ inline image ]  |
| @bob                               | | #121 10:43 You: ok                           | |                        |
| Other                              | |   ↳ Reply to Alice: looks good               | |                        |
| Project Logs                       | |                                              | |                        |
| Muted                              | |----------------------------------------------| |------------------------|
| Release Bot                        | | Input (Alt+Enter newline)                    | | Esc close, +/- zoom    |
+------------------------------------+ +----------------------------------------------+ +------------------------+
```

## Keyboard Shortcuts

### Global / Navigation
- `Ctrl+Q`: Quit
- `Ctrl+R`: Refresh chat list
- `Tab` / `Shift+Tab`: Cycle focus
- `Alt+Left/Right`: Focus previous/next pane
- `Ctrl+Left/Right`: Resize sidebar width
- `Up/Down`: Navigate chat list or input history
- `Enter`: Open selected chat / send message
- `Alt+Enter`: Insert newline in input
- `Esc`: Cancel reply mode (or close inline preview)

### Pane Management
- `Ctrl+V`: Split vertical
- `Ctrl+B`: Split horizontal
- `Ctrl+K`: Toggle split direction
- `Ctrl+W`: Close active pane
- `Ctrl+L`: Clear active pane
- `PageUp/PageDown`: Scroll messages

### Display & Chat Controls
- `Ctrl+E`: Toggle reactions
- `Ctrl+N`: Toggle desktop notifications
- `Ctrl+D`: Toggle compact mode
- `Ctrl+O`: Toggle emojis
- `Ctrl+G`: Toggle line numbers
- `Ctrl+T`: Toggle timestamps
- `Ctrl+S`: Toggle chat list
- `Ctrl+M`: Toggle unread count
- `Ctrl+P`: Mute/unmute selected chat
- `Ctrl+Y`: Toggle borders

### Inline Preview (Kitty)
- `Esc`: Close preview
- `+` / `=`: Zoom in
- `-`: Zoom out
- `n` or `Right`: Next image
- `p` or `Left`: Previous image

## Commands

- `/reply <N>` or `/r <N>`: Reply to message #N
- `/search <query>` or `/s <query>`: Search current chat
- `/media <N>` or `/m <N>`: Download/open media from message #N
  - In Kitty: inline preview for images
- `/edit <N> <text>` or `/e <N> <text>`: Edit message #N
- `/delete <N>` or `/d <N>`: Delete message #N
- `/alias <N> <name>`: Alias sender of message #N
- `/unalias <N>`: Remove alias for sender of message #N
- `/filter <type|sender>`: Filter by media/sender/link
- `/filter off`: Disable filter
- `/new @username`: Open DM by username
- `/newgroup <name>`: Create group
- `/add @username`: Add user to current group
- `/kick @username` or `/remove @username`: Remove user
- `/members`: List current group members
- `/forward <N> @username` or `/fwd <N> @username`: Forward message

## Install & Run

```bash
cd telegram_client_rs
cargo build --release
./target/release/telegram_client_rs
# or
cargo run --release
```

First run requires Telegram API credentials from `https://my.telegram.org`.

## Project Structure

```text
src/
  main.rs          Event loop, key/mouse input handling
  app.rs           Main UI + state + pane/chat behavior
  commands.rs      Slash commands
  telegram.rs      Telegram API integration
  split_view.rs    Pane tree and rendering split logic
  formatting.rs    Message formatting and wrapping
  persistence.rs   Layout/settings/aliases persistence
  kitty_preview.rs Kitty graphics protocol rendering
  utils.rs         Notifications, autocomplete, helpers
  widgets.rs       Chat/message data structures
```

## Configuration Files

### `telegram_config.json`

```json
{
  "api_id": 123456,
  "api_hash": "your_hash_here"
}
```

### `telegram_aliases.json`

```json
{
  "123456789": "Alice",
  "987654321": "Bob"
}
```

### `telegram_layout.json`

Saved automatically with pane layout, focused pane, and muted chat state.

## Development

```bash
cargo build
cargo test
cargo check
cargo fmt
cargo clippy
```

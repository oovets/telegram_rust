# Telegram Client - Rust TUI

A fully functional Telegram client in the terminal, written in Rust for maximum performance and responsiveness.

## Features

### Complete Implementation
- **Split View System**: Split screen vertically/horizontally into multiple panes
- **Multi-Chat Support**: Open multiple chats simultaneously in different panes
- **Click-to-Focus**: Click on panes to activate them, click on chats to open
- **Reply System**: Reply to messages with full context and quoted text
- **Message Formatting**: 
  - Color-coded messages (green for outgoing, cyan for incoming)
  - Red highlighting for replies to your own messages
  - Emoji support and URL shortening
  - Reaction display
- **Display Toggles**:
  - Ctrl+E: Reactions
  - Ctrl+O: Emojis
  - Ctrl+T: Timestamps
  - Ctrl+G: Line numbers
  - Ctrl+D: Compact mode
  - Ctrl+S: Chat list (show/hide sidebar)
  - Ctrl+Y: Toggle borders
- **Pane Management**:
  - Ctrl+V: Split vertically
  - Ctrl+B: Split horizontally
  - Ctrl+K: Toggle split direction (vertical ↔ horizontal)
  - Ctrl+W: Close pane
  - Ctrl+L: Clear pane
  - Tab/Shift+Tab: Cycle focus between panes and chat list
  - Ctrl+Left/Right: Switch directly between panes
  - Alt+Enter: Multi-line input
- **Commands**: /reply, /search, /media, /edit, /delete, /alias, /filter, /new, /newgroup, /add, /kick, /members, /forward
- **Persistence**: Saves layout, settings and aliases between sessions
- **Mouse Support**: Click to select panes and open chats

## Project Structure

```
telegram_client_rs/
├── Cargo.toml           # Dependencies and project configuration
├── src/
│   ├── main.rs         # Entry point, event loop and mouse handling
│   ├── app.rs          # Main application, UI logic and pane management
│   ├── config.rs       # Configuration management
│   ├── telegram.rs     # Telegram API integration (grammers-client)
│   ├── widgets.rs      # ChatPane, MessageData structures
│   ├── split_view.rs   # Split view tree structure and rendering
│   ├── commands.rs     # Command parser and handlers
│   ├── formatting.rs   # Message formatting, wrapping and URL handling
│   ├── persistence.rs  # Layout, settings and alias persistence
│   └── utils.rs        # Utility functions and desktop notifications
```

## Dependencies

- **ratatui** (0.29): Modern TUI framework
- **crossterm**: Cross-platform terminal manipulation and mouse events
- **tokio**: Async runtime
- **grammers-client**: Telegram MTProto client
- **grammers-session**: Session management
- **serde + serde_json**: Serialization for config and persistence
- **chrono**: Timestamp handling
- **anyhow**: Ergonomic error handling

## Installation & Running

```bash
# Clone repo
cd telegram_client_rs

# First time: requires Telegram API credentials
# Add api_id and api_hash to telegram_config.json

# Build
cargo build --release

# Run
./target/release/telegram_client_rs
# or
cargo run --release
```

## Usage

### Navigation
- **Up/Down**: Navigate in chat list or input history
- **Tab**: Cycle between chat list -> Pane 1 -> Pane 2 -> ... -> back to chat list
- **Shift+Tab**: Cycle focus backwards
- **Ctrl+Left/Right**: Switch directly between panes
- **Enter**: Open selected chat (in active pane) or send message
- **Alt+Enter**: Insert newline in input box
- **ESC**: Cancel reply mode, or return to chat list

### Mouse
- **Click on pane**: Activate that pane (green border) and focus input box
- **Click on chat**: Open chat in active pane

### Pane Management
- **Ctrl+V**: Split active pane vertically
- **Ctrl+B**: Split active pane horizontally
- **Ctrl+K**: Toggle split direction (switch between vertical and horizontal)
- **Ctrl+W**: Close active pane
- **Ctrl+L**: Clear active pane
- **PageUp/PageDown**: Scroll messages
- **Click on pane**: Activate pane and focus input box

### Display Settings
- **Ctrl+E**: Toggle reactions
- **Ctrl+N**: Toggle notifications
- **Ctrl+D**: Toggle compact mode
- **Ctrl+O**: Toggle emojis
- **Ctrl+G**: Toggle line numbers
- **Ctrl+T**: Toggle timestamps
- **Ctrl+S**: Toggle chat list (sidebar)
- **Ctrl+Y**: Toggle borders


### Commands
Type in the input field:
- `/reply <N>` or `/r <N>`: Reply to message #N (set reply mode or inline reply with `/r N text`)
- `/search <query>` or `/s <query>`: Search messages in active chat
- `/media <N>` or `/m <N>`: Download and open media from message #N
- `/edit <N> <text>` or `/e <N> <text>`: Edit message #N
- `/delete <N>` or `/d <N>`: Delete message #N
- `/alias <N> <name>`: Set display alias for sender of message #N
- `/unalias <N>`: Remove alias for sender of message #N
- `/filter <type>`: Filter messages (photo, video, audio, doc, link, sticker, or sender name)
- `/filter off`: Disable filter
- `/new @username`: Open a DM with a user by username
- `/newgroup <name>`: Create a new group chat
- `/add @username`: Add a user to the current group
- `/kick @username` or `/remove @username`: Remove a user from the current group
- `/members`: List members of the current group
- `/forward <N> @username` or `/fwd <N> @username`: Forward message #N to a user/chat

### Shortcuts
- **Ctrl+Q**: Quit
- **Ctrl+R**: Refresh chat list
- **Ctrl+I**: Input focus toggle (legacy)

## File Formats

### telegram_config.json
```json
{
  "api_id": 123456,
  "api_hash": "your_hash_here",
  "session_file": "telegram_session.session"
}
```

### telegram_aliases.json
```json
{
  "123456789": "Alice",
  "987654321": "Bob"
}
```

### telegram_layout.json
Automatically saves split layout and pane configuration between sessions.

### Planned
- Typing indicators
- Online status
- Message search pagination

## Development

```bash
# Debug build with logging
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test

# Check code without building
cargo check

# Format code
cargo fmt

# Lint code
cargo clippy
```

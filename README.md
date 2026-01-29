# Telegram Terminal Client

A terminal/TUI client for Telegram built with Python, Textual and Telethon.

## Features

- Full terminal/TUI client (no GUI, works over SSH)
- Split panes - view multiple conversations simultaneously
- Media download and viewing (photos, videos, documents, etc.)
- Two-factor authentication support
- Layout persistence (remembers your splits and open chats)
- URL shortening for long links
- Message reactions display
- YouTube and Spotify link detection with title preview
- User aliases (shorten long names)
- Message filtering (by sender, media type, links)
- Search in message history
- Desktop notifications (macOS/Linux)
- DISABLE EMOJIS IN CHATS!

..and many others!

## Installation

1. Install dependencies:
```bash
pip install -r requirements.txt
```

2. Get API credentials from Telegram:
   - Go to https://my.telegram.org
   - Log in with your phone number
   - Create a new application
   - Copy the API ID and API Hash

## Usage

1. Run the client:
```bash
python telegram_client.py
```

2. On first run, enter your API ID and API Hash when prompted

3. Follow the instructions to log in:
   - Enter your phone number
   - Enter the verification code sent to Telegram
   - If you have two-factor authentication, enter your password

4. Select a conversation from the sidebar to view messages

5. Type messages in the input field and press Enter to send

## Keybindings

| Key | Action |
|-----|--------|
| `Ctrl+V` | Split vertically (side by side) |
| `Ctrl+B` | Split horizontally (stacked) |
| `Ctrl+W` | Close current pane |
| `Tab` | Cycle between panes |
| `Ctrl+R` | Refresh conversations |
| `Ctrl+L` | Clear current pane |
| `Ctrl+E` | Toggle reactions on/off |
| `Ctrl+N` | Toggle desktop notifications |
| `Ctrl+D` | Toggle compact mode (spacing between messages) |
| `Ctrl+O` | Toggle emojis on/off |
| `Ctrl+Q` | Quit (saves layout) |
| `Tab` | Autocomplete commands (when typing /) |

## Commands

| Command | Description |
|---------|-------------|
| `/reply N` | Reply to message #N |
| `/media N` or `/m N` | Download and open media from message #N |
| `/edit N text` or `/e N text` | Edit your message #N with new text |
| `/delete N` or `/del N` or `/d N` | Delete your message #N |
| `/alias N name` | Set display alias for sender of message #N |
| `/unalias N` | Remove alias for sender of message #N |
| `/filter <type>` | Filter messages (photo/video/audio/doc/link/name) |
| `/filter off` | Disable filter |
| `/search <query>` or `/s <query>` | Search message history |
| `/new @username` | Start new chat with user |
| `/newgroup <name>` | Create a new group |
| `/add @username` | Add member to current group |
| `/kick @username` or `/remove @username` | Remove member from group |
| `/members` | List members in current group |
| `/forward N @target` or `/fwd N @target` | Forward message #N to @target |

## Configuration Files

| File | Description |
|------|-------------|
| `telegram_config.json` | API credentials (saved automatically) |
| `telegram_session.session` | Session data (no need to log in every time) |
| `telegram_layout.json` | Window layout and open chats (saved on quit) |
| `telegram_aliases.json` | User display name aliases |

## Security

- Session file contains sensitive data - do not share it
- API credentials are stored locally in plain text - protect these files
- Do not use this client on shared computers without removing the session file after use

## Technical Information

- **Telethon**: Telegram API library for Python
- **Textual**: Modern TUI framework for Python
- **Asyncio**: For asynchronous communication with Telegram servers

## Default Settings

- Shows up to 200 conversations and 100 messages per conversation (configurable in code)

## Future Improvements

- [ ] Voice message playback
- [ ] Pin/unpin messages
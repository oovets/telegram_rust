# Telegram Terminal Client

A terminal/TUI client for Telegram built with Python, Textual and Telethon.

## Features

- Full terminal/TUI client (no GUI, works over SSH)
- Modern terminal UI with Textual framework
- Split panes - view multiple conversations simultaneously
- Real-time message sending and receiving
- Message history with reply support
- Media download and viewing (photos, videos, documents, etc.)
- Session management (saves login)
- Two-factor authentication support
- Layout persistence (remembers your splits and open chats)
- Word-wrap at word boundaries for better readability
- URL shortening for long links

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
| `Ctrl+Q` | Quit (saves layout) |

## Commands

| Command | Description |
|---------|-------------|
| `/reply N` | Reply to message #N |
| `/media N` or `/m N` | Download and open media from message #N |

## Configuration Files

| File | Description |
|------|-------------|
| `telegram_config.json` | API credentials (saved automatically) |
| `telegram_session.session` | Session data (no need to log in every time) |
| `telegram_layout.json` | Window layout and open chats (saved on quit) |

## Security

- Session file contains sensitive data - do not share it
- API credentials are stored locally in plain text - protect these files
- Do not use this client on shared computers without removing the session file after use

## Technical Information

- **Telethon**: Telegram API library for Python
- **Textual**: Modern TUI framework for Python
- **Asyncio**: For asynchronous communication with Telegram servers

## Current Limitations

- Limited to 100 conversations and 50 message history per conversation
- No search functionality yet
- No message editing/deletion yet

## Future Improvements

- [ ] Search functionality
- [ ] Group management
- [ ] Message filtering
- [ ] Desktop notifications
- [ ] Message editing and deletion
- [ ] Inline media preview (ASCII art)

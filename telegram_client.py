#!/usr/bin/env python3
"""
Telegram Terminal Client - A TUI client for Telegram.

A modern terminal user interface for Telegram built with Python,
Textual framework and Telethon library.

Features:
    - Split panes for multiple conversations
    - Real-time message updates
    - Media download support
    - Layout persistence
    - Reply support

Usage:
    python telegram_client.py

"""

__version__ = "1.0.0"

import asyncio
import json
import os
import re
import threading
import urllib.parse
import urllib.request
from datetime import datetime
from typing import Dict, List, Optional

from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, ScrollableContainer, Vertical
from textual.message import Message
from textual.widgets import Header, Input, ListItem, ListView, Static

from telethon import TelegramClient, events
from telethon.tl.types import Channel, Chat, User

# Configuration
DEBUG_MODE = os.environ.get("TELEGRAM_DEBUG", "").lower() in ("1", "true", "yes")

try:
    _script_dir = os.path.dirname(os.path.abspath(__file__))
except NameError:
    _script_dir = os.getcwd()

LOG_FILE = os.path.join(_script_dir, "telegram_client.log")


def _log(message: str, level: str = "DEBUG") -> None:
    """Write a log message to file (only errors by default, all if DEBUG_MODE)."""
    if level == "ERROR" or DEBUG_MODE:
        try:
            with open(LOG_FILE, "a", encoding="utf-8") as f:
                timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
                f.write(f"[{timestamp}] {level}: {message}\n")
        except OSError:
            pass


# =============================================================================
# CSS Styles
# =============================================================================

APP_CSS = """
#sidebar {
    width: 30;
    border-right: solid grey;
    background: #1e1e1e;
}

.sidebar-header {
    padding: 0 1;
    text-style: bold;
    color: white;
    height: 1;
}

#chat-list {
    height: 1fr;
    scrollbar-size: 0 0;
    background: #1e1e1e;
}

ChatListItem {
    height: 1;
    padding: 0 1;
}

ListView > .list-view--highlight {
    background: grey;
}

#pane-container {
    width: 1fr;
    background: #1e1e1e;
}

SplitContainer {
    width: 1fr;
    height: 1fr;
    background: #1e1e1e;
}

ChatPane {
    height: 1fr;
    width: 1fr;
    border-left: solid #333333;
    background: #1e1e1e;
}

ChatPane:first-child {
    border-left: none;
}

.pane-header {
    padding: 0 1;
    text-style: bold;
    color: white;
    height: 1;
}

.pane-messages-container {
    height: 1fr;
    padding: 0 1;
    scrollbar-size: 0 0;
    background: #1e1e1e;
}

.pane-messages {
    width: 1fr;
}

.pane-input-container {
    height: auto;
    min-height: 2;
    max-height: 5;
    border-top: solid grey;
    background: #1e1e1e;
}

.pane-reply-preview {
    width: auto;
    max-width: 50;
    height: auto;
    display: none;
    text-style: italic;
    color: grey;
    padding: 0 1;
}

.pane-reply-preview.visible {
    display: block;
}

.pane-input {
    width: 1fr;
    height: 2;
    min-height: 2;
    border: none;
}

.pane-input:focus {
    border: none;
}

.pane-focused {
    border-left: solid #666666;
}
"""


# =============================================================================
# Message Classes
# =============================================================================

class ChatListItem(ListItem):
    """A list item representing a chat in the sidebar."""

    def __init__(self, chat_id: int, chat_info: Dict, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.chat_id = chat_id
        self.chat_info = chat_info

    def compose(self):
        name = self.chat_info['name']
        unread = self.chat_info.get('unread', 0)
        username = self.chat_info.get('username', '')
        safe_name = name.replace("[", "\\[")
        if unread > 0:
            display_text = f"[reverse] {safe_name} ({unread}) [/reverse]"
        else:
            display_text = f" {safe_name}"
        if username:
            display_text += f" {username}"
        yield Static(display_text)


class ChatSelected(Message):
    """Message sent when a chat is selected from the sidebar."""

    def __init__(self, chat_id: int):
        super().__init__()
        self.chat_id = chat_id


class PaneClicked(Message):
    """Message sent when a pane is clicked."""

    def __init__(self, pane: "ChatPane"):
        super().__init__()
        self.pane = pane


# =============================================================================
# Widget Classes
# =============================================================================

class SplitContainer(Horizontal):
    """A container that holds panes or other split containers for split views."""

    _split_counter = 0

    def __init__(self, direction: str = "horizontal", *args, **kwargs):
        SplitContainer._split_counter += 1
        self.split_id = f"split-{SplitContainer._split_counter}"
        super().__init__(*args, id=self.split_id, **kwargs)
        self.direction = direction
        if direction == "vertical":
            self.styles.layout = "vertical"
        else:
            self.styles.layout = "horizontal"


class ChatPane(Vertical):
    """A single chat pane with its own header, messages, and input field."""

    _pane_counter = 0

    def __init__(self, *args, **kwargs):
        ChatPane._pane_counter += 1
        self.pane_id = f"pane-{ChatPane._pane_counter}"
        super().__init__(*args, id=self.pane_id, **kwargs)
        self.chat_id: Optional[int] = None
        self.msg_data: List = []
        self.reply_to_message: Optional[int] = None

    def compose(self):
        yield Static("", classes="pane-header")
        yield ScrollableContainer(
            Static("Select a conversation...", classes="pane-messages"),
            classes="pane-messages-container"
        )
        with Horizontal(classes="pane-input-container"):
            yield Static("", classes="pane-reply-preview")
            yield Input(placeholder="Type a message... (/reply N to reply)", classes="pane-input")

    def set_chat_header(self, name: str, username: str = "") -> None:
        """Update the pane header with chat name."""
        header = self.query_one(".pane-header", Static)
        text = name
        if username:
            text += f" {username}"
        header.update(text)

    def set_messages(self, text: str) -> None:
        """Update the messages display and scroll to bottom."""
        content = self.query_one(".pane-messages", Static)
        content.update(text)

        def scroll(_delay=None):
            try:
                container = self.query_one(".pane-messages-container", ScrollableContainer)
                container.scroll_end(animate=False)
            except Exception:
                pass

        self.call_later(scroll, 0.2)

    def show_reply_preview(self, text: str) -> None:
        """Show the reply preview bar."""
        try:
            rp = self.query_one(".pane-reply-preview", Static)
            rp.update(text)
            rp.add_class("visible")
        except Exception:
            pass

    def hide_reply_preview(self) -> None:
        """Hide the reply preview bar."""
        try:
            rp = self.query_one(".pane-reply-preview", Static)
            rp.update("")
            rp.remove_class("visible")
        except Exception:
            pass

    def get_input(self) -> Optional[Input]:
        """Get the input widget for this pane."""
        try:
            return self.query_one(".pane-input", Input)
        except Exception:
            return None

    def on_click(self, event) -> None:
        """When pane is clicked, notify app to make it active."""
        self.post_message(PaneClicked(self))


class ChatList(Vertical):
    """Sidebar widget displaying the list of chats."""

    BINDINGS = [
        Binding("j", "cursor_down", "Down", show=False),
        Binding("k", "cursor_up", "Up", show=False),
        Binding("enter", "select_chat", "Select", show=False),
    ]

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.chat_list_view: Optional[ListView] = None
        self.chats: Dict[int, Dict] = {}
        self.chat_items: Dict[int, ChatListItem] = {}

    def compose(self):
        yield Static("Chats", classes="sidebar-header")
        yield ListView(id="chat-list")

    def on_mount(self) -> None:
        self.chat_list_view = self.query_one("#chat-list", ListView)

    def on_list_view_selected(self, event: ListView.Selected) -> None:
        if isinstance(event.item, ChatListItem):
            self.post_message(ChatSelected(event.item.chat_id))

    def update_chats(self, chats: Dict[int, Dict]) -> None:
        """Update the chat list with new data."""
        self.chats = chats
        self.chat_items.clear()

        try:
            list_view = self.query_one("#chat-list", ListView)
            if list_view is not None:
                self.chat_list_view = list_view
            else:
                return
        except Exception:
            return

        try:
            self.chat_list_view.clear()
            if len(chats) == 0:
                return

            def get_sort_key(item):
                chat_info = item[1]
                last_msg = chat_info.get('last_message')
                if last_msg and hasattr(last_msg, 'date'):
                    return last_msg.date
                return datetime.min.replace(tzinfo=None)

            sorted_chats = sorted(chats.items(), key=get_sort_key, reverse=True)
            for chat_id, chat_info in sorted_chats:
                try:
                    item = ChatListItem(chat_id, chat_info)
                    self.chat_items[chat_id] = item
                    self.chat_list_view.append(item)
                except Exception:
                    pass

            self.chat_list_view.refresh()
        except Exception as e:
            _log(f"Failed to update chat list: {e}", "ERROR")

    def get_selected_chat_id(self) -> Optional[int]:
        """Get the currently highlighted chat ID."""
        if not self.chat_list_view:
            return None
        highlighted = self.chat_list_view.highlighted
        if highlighted is not None and highlighted < len(self.chat_list_view.children):
            item = self.chat_list_view.children[highlighted]
            if isinstance(item, ChatListItem):
                return item.chat_id
        return None

    def action_cursor_down(self) -> None:
        if self.chat_list_view:
            self.chat_list_view.action_cursor_down()

    def action_cursor_up(self) -> None:
        if self.chat_list_view:
            self.chat_list_view.action_cursor_up()

    def action_select_chat(self) -> None:
        chat_id = self.get_selected_chat_id()
        if chat_id:
            self.post_message(ChatSelected(chat_id))


# =============================================================================
# Main Application
# =============================================================================

class TelegramApp(App):
    """Main Telegram TUI application."""

    CSS = APP_CSS

    BINDINGS = [
        Binding("ctrl+q", "quit", "Quit", priority=True),
        Binding("ctrl+r", "refresh", "Refresh", priority=True),
        Binding("ctrl+l", "clear", "Clear", priority=True),
        Binding("tab", "cycle_pane", "Next pane", priority=True),
        Binding("ctrl+v", "split_vertical", "Split V", priority=True),
        Binding("ctrl+b", "split_horizontal", "Split H", priority=True),
        Binding("ctrl+w", "close_pane", "Close pane", priority=True),
    ]

    TITLE = "Telegram Terminal Client"

    def __init__(self):
        super().__init__()
        script_dir = os.path.dirname(os.path.abspath(__file__))
        self.session_file = os.path.join(script_dir, "telegram_session.session")
        self.config_file = os.path.join(script_dir, "telegram_config.json")
        self.layout_file = os.path.join(script_dir, "telegram_layout.json")

        self.api_id = None
        self.api_hash = None
        self.client: Optional[TelegramClient] = None
        self.telegram_loop: Optional[asyncio.AbstractEventLoop] = None
        self.telegram_thread: Optional[threading.Thread] = None

        self.chats: Dict[int, Dict] = {}
        self.messages: Dict[int, List] = {}
        self.running = True

        self.panes: list = []
        self.active_pane: Optional[ChatPane] = None
        self.saved_layout: Optional[dict] = None

        self.load_config()
        self.load_layout()

    def load_config(self):
        if os.path.exists(self.config_file):
            try:
                with open(self.config_file, 'r') as f:
                    config = json.load(f)
                    self.api_id = config.get('api_id')
                    self.api_hash = config.get('api_hash')
            except Exception:
                pass

    def save_config(self):
        try:
            with open(self.config_file, 'w') as f:
                json.dump({'api_id': self.api_id, 'api_hash': self.api_hash}, f)
        except Exception:
            pass

    def load_layout(self) -> None:
        """Load saved layout from file."""
        if os.path.exists(self.layout_file):
            try:
                with open(self.layout_file, 'r') as f:
                    self.saved_layout = json.load(f)
            except (json.JSONDecodeError, OSError) as e:
                _log(f"Failed to load layout: {e}", "ERROR")
                self.saved_layout = None

    def save_layout(self) -> None:
        """Save current layout to file."""
        try:
            layout = self._serialize_layout()
            with open(self.layout_file, 'w') as f:
                json.dump(layout, f, indent=2)
        except (OSError, TypeError) as e:
            _log(f"Failed to save layout: {e}", "ERROR")

    def _serialize_layout(self) -> dict:
        """Serialize the current pane layout to a dict."""
        container = self.query_one("#pane-container")
        return {
            "version": 1,
            "tree": self._serialize_node(container),
            "active_pane_chat_id": self.active_pane.chat_id if self.active_pane else None
        }

    def _serialize_node(self, node) -> dict:
        """Recursively serialize a node (container or pane)."""
        if isinstance(node, ChatPane):
            return {
                "type": "pane",
                "chat_id": node.chat_id
            }
        elif isinstance(node, SplitContainer):
            return {
                "type": "split",
                "direction": node.direction,
                "children": [self._serialize_node(child) for child in node.children]
            }
        elif hasattr(node, 'children'):
            # Root container
            children = [self._serialize_node(child) for child in node.children]
            return {
                "type": "root",
                "children": children
            }
        return {"type": "unknown"}

    def _restore_layout(self) -> None:
        """Restore layout from saved_layout after chats are loaded."""
        if not self.saved_layout or "tree" not in self.saved_layout:
            return

        tree = self.saved_layout["tree"]
        active_chat_id = self.saved_layout.get("active_pane_chat_id")

        # Clear existing panes
        for pane in self.panes[:]:
            pane.remove()
        self.panes.clear()
        self.active_pane = None

        # Restore from tree
        container = self.query_one("#pane-container")
        self._restore_node(tree, container)

        # Set active pane
        if active_chat_id and self.panes:
            for pane in self.panes:
                if pane.chat_id == active_chat_id:
                    self._set_active_pane(pane)
                    break
            else:
                self._set_active_pane(self.panes[0])
        elif self.panes:
            self._set_active_pane(self.panes[0])

        # Load messages for all panes with chat_ids
        def load_pane_messages(_delay=None):
            for pane in self.panes:
                if pane.chat_id and pane.chat_id in self.chats:
                    self.schedule_load_messages(pane.chat_id, pane)
        self.call_later(load_pane_messages, 0.5)

    def _restore_node(self, node_data: dict, parent):
        """Recursively restore a node from serialized data."""
        node_type = node_data.get("type")

        if node_type == "pane":
            pane = ChatPane()
            pane.chat_id = node_data.get("chat_id")
            parent.mount(pane)
            self.panes.append(pane)
            return pane

        elif node_type == "split":
            direction = node_data.get("direction", "horizontal")
            split = SplitContainer(direction=direction)
            parent.mount(split)
            for child_data in node_data.get("children", []):
                self._restore_node(child_data, split)
            return split

        elif node_type == "root":
            for child_data in node_data.get("children", []):
                self._restore_node(child_data, parent)
            return parent

        return None

    def compose(self) -> ComposeResult:
        yield Header(show_clock=True)
        with Horizontal():
            yield ChatList(id="sidebar")
            yield Horizontal(id="pane-container")

    async def on_mount(self):
        # Add initial pane only if no saved layout
        if not self.saved_layout:
            self._add_pane()
        await self.connect_telegram()
        def focus_input(_delay=None):
            if self.active_pane:
                inp = self.active_pane.get_input()
                if inp:
                    inp.focus()
        self.call_later(focus_input, 0.5)

    def _add_pane(self) -> ChatPane:
        pane = ChatPane()
        container = self.query_one("#pane-container")
        container.mount(pane)
        self.panes.append(pane)
        self._set_active_pane(pane)
        return pane

    def _set_active_pane(self, pane: ChatPane):
        if self.active_pane:
            self.active_pane.remove_class("pane-focused")
        self.active_pane = pane
        pane.add_class("pane-focused")
        def focus_pane_input(_delay=None):
            try:
                inp = pane.get_input()
                if inp:
                    inp.focus()
            except Exception:
                pass
        self.call_later(focus_pane_input, 0.1)

    def _find_pane_for_input(self, input_widget: Input) -> Optional[ChatPane]:
        for pane in self.panes:
            if pane.get_input() is input_widget:
                return pane
        return None

    async def connect_telegram(self) -> None:
        """Connect to Telegram in a background thread."""
        if not self.api_id or not self.api_hash:
            await self.show_api_dialog()
            return

        def run_telegram_loop():
            self.telegram_loop = asyncio.new_event_loop()
            asyncio.set_event_loop(self.telegram_loop)
            try:
                self.telegram_loop.run_until_complete(self._connect_worker())
                self.telegram_loop.run_forever()
            except Exception as e:
                _log(f"Telegram loop error: {e}", "ERROR")

        self.telegram_thread = threading.Thread(target=run_telegram_loop, daemon=True)
        self.telegram_thread.start()

    async def show_api_dialog(self) -> None:
        """Show API credentials dialog."""
        self.notify("Enter API credentials in the terminal", severity="warning")

    async def _connect_worker(self) -> None:
        """Worker coroutine for Telegram connection."""
        try:
            self.client = TelegramClient(self.session_file, self.api_id, self.api_hash)
            await self.client.start()

            is_authorized = await self.client.is_user_authorized()
            if not is_authorized:
                self.notify("Not logged in. Check terminal.", severity="warning")
                return

            me = await self.client.get_me()
            self.call_from_thread(lambda: self.notify(f"Logged in as {me.first_name}", severity="success"))

            await self.load_conversations()
            self.client.add_event_handler(self.on_new_message, events.NewMessage)
        except Exception as e:
            _log(f"Connection error: {e}", "ERROR")

    async def load_conversations(self):
        if not self.client:
            return
        try:
            dialogs = await self.client.get_dialogs(limit=100)
            self.chats.clear()
            for dialog in dialogs:
                entity = dialog.entity
                chat_id = entity.id
                if isinstance(entity, User):
                    name = entity.first_name or ""
                    if entity.last_name:
                        name += f" {entity.last_name}"
                    if not name:
                        name = f"User {chat_id}"
                    username = f"@{entity.username}" if entity.username else ""
                elif isinstance(entity, (Chat, Channel)):
                    name = entity.title or f"Chat {chat_id}"
                    username = f"@{entity.username}" if hasattr(entity, 'username') and entity.username else ""
                else:
                    name = f"Unknown {chat_id}"
                    username = ""
                self.chats[chat_id] = {
                    'name': name, 'username': username, 'entity': entity,
                    'unread': dialog.unread_count, 'last_message': dialog.message
                }
            chats_dict = self.chats.copy()
            def update_sidebar_and_restore():
                try:
                    sidebar = self.query_one("#sidebar", ChatList)
                    sidebar.update_chats(chats_dict)
                    # Restore layout after chats are loaded
                    if self.saved_layout and len(self.panes) == 0:
                        self._restore_layout()
                        self.saved_layout = None  # Only restore once
                except Exception as e:
                    _log(f"UI update failed: {e}", "ERROR")
            self.call_from_thread(update_sidebar_and_restore)
        except Exception as e:
            _log(f"Failed to load conversations: {e}", "ERROR")

    def on_chat_selected(self, event: ChatSelected):
        if not self.active_pane:
            self._add_pane()
        pane = self.active_pane
        pane.chat_id = event.chat_id
        pane.reply_to_message = None
        pane.hide_reply_preview()
        self.schedule_load_messages(event.chat_id, pane)

    def _mark_chat_read(self, chat_id: int):
        if chat_id in self.chats:
            self.chats[chat_id]['unread'] = 0
        try:
            chat_list = self.query_one("#chat-list", ListView)
            for item in chat_list.children:
                if hasattr(item, 'chat_id') and item.chat_id == chat_id:
                    name = self.chats[chat_id]['name']
                    safe_name = name.replace("[", "\\[")
                    username = self.chats[chat_id].get('username', '')
                    display_text = f" {safe_name}"
                    if username:
                        display_text += f" {username}"
                    item.query_one(Static).update(display_text)
                    break
        except Exception as e:
            _log(f"Failed to mark chat read: {e}", "ERROR")

    def _wrap_text(self, text: str, indent: int, width: int) -> str:
        """Wrap text at word boundaries, never breaking mid-word."""
        if width <= indent:
            return text
        content_width = width - indent
        pad = " " * indent
        result_lines = []
        
        for i, paragraph in enumerate(text.split("\n")):
            if not paragraph:
                result_lines.append(pad if i > 0 else "")
                continue
            
            words = paragraph.split(" ")
            current_line = ""
            first_line_of_para = (i == 0)
            
            for word in words:
                # Handle very long words (longer than content_width)
                if len(word) > content_width:
                    # Flush current line first
                    if current_line:
                        if first_line_of_para and len(result_lines) == 0:
                            result_lines.append(current_line)
                        else:
                            result_lines.append(pad + current_line)
                        current_line = ""
                    # Break long word into chunks
                    for j in range(0, len(word), content_width):
                        chunk = word[j:j + content_width]
                        if first_line_of_para and len(result_lines) == 0:
                            result_lines.append(chunk)
                        else:
                            result_lines.append(pad + chunk)
                    first_line_of_para = False
                    continue
                
                # Check if word fits on current line
                test_line = current_line + (" " if current_line else "") + word
                if len(test_line) <= content_width:
                    current_line = test_line
                else:
                    # Flush current line and start new one
                    if current_line:
                        if first_line_of_para and len(result_lines) == 0:
                            result_lines.append(current_line)
                        else:
                            result_lines.append(pad + current_line)
                        first_line_of_para = False
                    current_line = word
            
            # Flush remaining content
            if current_line:
                if first_line_of_para and len(result_lines) == 0:
                    result_lines.append(current_line)
                else:
                    result_lines.append(pad + current_line)
        
        return "\n".join(result_lines)

    def _shorten_url(self, url: str) -> str:
        try:
            api_url = f"https://is.gd/create.php?format=simple&url={urllib.parse.quote(url, safe='')}"
            req = urllib.request.Request(api_url, headers={"User-Agent": "TelegramTUI/1.0"})
            with urllib.request.urlopen(req, timeout=3) as resp:
                short = resp.read().decode().strip()
                if short.startswith("http"):
                    return short
        except Exception:
            pass
        return url

    _url_cache: dict = {}

    def _shorten_urls_in_text(self, text: str) -> str:
        url_pattern = re.compile(r'https?://\S{40,}')
        def replacer(match):
            url = match.group(0)
            if url not in self._url_cache:
                self._url_cache[url] = self._shorten_url(url)
            return self._url_cache[url]
        return url_pattern.sub(replacer, text)

    def _get_media_label(self, msg) -> str:
        if msg.photo:
            return "[Photo]"
        elif msg.video:
            return "[Video]"
        elif msg.audio:
            return "[Audio]"
        elif msg.voice:
            return "[Voice message]"
        elif msg.video_note:
            return "[Video note]"
        elif msg.sticker:
            return "[Sticker]"
        elif msg.gif:
            return "[GIF]"
        elif msg.document:
            name = ""
            if hasattr(msg.document, 'attributes'):
                for attr in msg.document.attributes:
                    if hasattr(attr, 'file_name') and attr.file_name:
                        name = attr.file_name
                        break
            return f"[Document: {name}]" if name else "[Document]"
        elif msg.contact:
            return "[Contact]"
        elif msg.geo:
            return "[Location]"
        elif msg.poll:
            return "[Poll]"
        return ""

    def _format_messages(self, msg_data: list, pane: ChatPane) -> str:
        try:
            container = pane.query_one(".pane-messages-container", ScrollableContainer)
            width = container.size.width - 2
        except Exception:
            width = 80
        if width < 20:
            width = 80

        lines = []
        for idx, item in enumerate(msg_data):
            msg, sender_name, is_out = item[0], item[1], item[2]
            reply_info = item[3] if len(item) > 3 else None

            media_label = self._get_media_label(msg)
            text = msg.text or ""
            if media_label:
                text = f"{media_label} {text}" if text else media_label
            if not text:
                continue

            timestamp = msg.date.strftime("%H:%M")
            safe_name = sender_name.replace("[", "\\[")
            num_str = f"#{idx + 1}"
            prefix_len = len(num_str) + 1 + len(timestamp) + 1 + len(sender_name) + 2
            text = self._shorten_urls_in_text(text)
            safe_text = text.replace("[", "\\[")
            wrapped = self._wrap_text(safe_text, prefix_len, width)

            if reply_info:
                reply_sender, reply_text = reply_info
                safe_reply_sender = reply_sender.replace("[", "\\[")
                safe_reply_text = reply_text.replace("[", "\\[")
                pad = " " * prefix_len
                lines.append(f"{pad}[dim italic]> {safe_reply_sender}: {safe_reply_text}[/dim italic]")

            num = f"[dim]{num_str}[/dim] "
            if is_out:
                lines.append(f"{num}[dim]{timestamp}[/dim] [bold green]{safe_name}[/bold green]: {wrapped}")
            else:
                lines.append(f"{num}[dim]{timestamp}[/dim] [bold cyan]{safe_name}[/bold cyan]: {wrapped}")
        return "\n".join(lines)

    def _display_messages_in_pane(self, chat_id: int, msg_data: list, pane: ChatPane):
        try:
            pane.msg_data = msg_data
            chat_info = self.chats[chat_id]
            pane.set_chat_header(chat_info['name'], chat_info['username'])
            pane.set_messages(self._format_messages(msg_data, pane))
        except Exception as e:
            _log(f"Display error: {e}", "ERROR")

    async def _resolve_sender_name(self, msg) -> str:
        if msg.out:
            return "You"
        try:
            sender = await msg.get_sender()
            if sender:
                if isinstance(sender, User):
                    name = sender.first_name or ""
                    if sender.last_name:
                        name += f" {sender.last_name}"
                    return name.strip() if name.strip() else "Unknown"
                elif hasattr(sender, 'title') and sender.title:
                    return sender.title
        except Exception:
            pass
        return "Unknown"

    async def _load_and_resolve(self, entity, chat_id: int):
        messages = await self.client.get_messages(entity, limit=50)
        msg_by_id = {m.id: m for m in messages}
        sender_cache = {}
        for msg in messages:
            sender_cache[msg.id] = await self._resolve_sender_name(msg)

        msg_data = []
        for msg in reversed(messages):
            sender_name = sender_cache[msg.id]
            reply_info = None
            if msg.reply_to and hasattr(msg.reply_to, 'reply_to_msg_id') and msg.reply_to.reply_to_msg_id:
                reply_id = msg.reply_to.reply_to_msg_id
                if reply_id in msg_by_id:
                    reply_msg = msg_by_id[reply_id]
                    reply_sender = sender_cache.get(reply_id, "Unknown")
                    reply_text = reply_msg.text[:40] if reply_msg.text else "[Media]"
                    if reply_msg.text and len(reply_msg.text) > 40:
                        reply_text += "..."
                    reply_info = (reply_sender, reply_text)
                else:
                    try:
                        reply_msg = await self.client.get_messages(entity, ids=reply_id)
                        if reply_msg:
                            rs = await self._resolve_sender_name(reply_msg)
                            rt = reply_msg.text[:40] if reply_msg.text else "[Media]"
                            if reply_msg.text and len(reply_msg.text) > 40:
                                rt += "..."
                            reply_info = (rs, rt)
                    except Exception:
                        pass
            msg_data.append((msg, sender_name, msg.out, reply_info))
        self.messages[chat_id] = messages
        return msg_data

    def schedule_load_messages(self, chat_id: int, pane: ChatPane):
        if not self.client or chat_id not in self.chats or not self.telegram_loop:
            return
        entity = self.chats[chat_id]['entity']
        async def _load():
            try:
                msg_data = await self._load_and_resolve(entity, chat_id)
                await self.client.send_read_acknowledge(entity)
                self.call_from_thread(lambda: self._display_messages_in_pane(chat_id, msg_data, pane))
                self.call_from_thread(lambda: self._mark_chat_read(chat_id))
            except Exception as e:
                _log(f"Failed to load messages: {e}", "ERROR")
        asyncio.run_coroutine_threadsafe(_load(), self.telegram_loop)

    def on_input_submitted(self, event: Input.Submitted):
        pane = self._find_pane_for_input(event.input)
        if not pane:
            return

        self._set_active_pane(pane)
        text = event.value.strip()
        if not text:
            return

        if text.startswith("/reply "):
            self._handle_reply_command(text, pane)
            event.input.value = ""
            return

        if text.startswith("/media ") or text.startswith("/m "):
            self._handle_media_command(text, pane)
            event.input.value = ""
            return

        self._send_message(text, pane)
        event.input.value = ""
        pane.reply_to_message = None
        pane.hide_reply_preview()
        try:
            event.input.focus()
        except Exception:
            pass

    def on_input_changed(self, event: Input.Changed):
        """Track which pane gets focus when user types."""
        pane = self._find_pane_for_input(event.input)
        if pane and pane is not self.active_pane:
            self._set_active_pane(pane)

    def on_focus(self, event) -> None:
        """Track which pane gets focus when clicking input."""
        # Check if the focused widget is inside a ChatPane
        widget = event.widget
        
        # Walk up the DOM to find parent ChatPane
        current = widget
        while current is not None:
            if isinstance(current, ChatPane):
                if current is not self.active_pane:
                    # Don't call _set_active_pane to avoid focus loop, just update state
                    if self.active_pane:
                        self.active_pane.remove_class("pane-focused")
                    self.active_pane = current
                    current.add_class("pane-focused")
                return
            current = current.parent

    def on_pane_clicked(self, event: PaneClicked) -> None:
        """Handle click anywhere in a pane to make it active."""
        pane = event.pane
        if pane in self.panes and pane is not self.active_pane:
            if self.active_pane:
                self.active_pane.remove_class("pane-focused")
            self.active_pane = pane
            pane.add_class("pane-focused")
            # Focus the input
            inp = pane.get_input()
            if inp:
                inp.focus()

    def on_resize(self, event) -> None:
        """Redraw messages when window is resized."""
        def redraw_panes(_delay=None):
            for pane in self.panes:
                try:
                    if pane.chat_id and pane.msg_data:
                        pane.set_messages(self._format_messages(pane.msg_data, pane))
                except Exception:
                    pass
        self.call_later(redraw_panes, 0.1)

    def _handle_reply_command(self, text: str, pane: ChatPane):
        parts = text.split(maxsplit=1)
        if len(parts) < 2:
            self.notify("Usage: /reply N", severity="warning")
            return
        try:
            num = int(parts[1])
        except ValueError:
            self.notify("Usage: /reply N (where N is the message number)", severity="warning")
            return

        if not pane.msg_data:
            self.notify("No messages loaded", severity="warning")
            return

        text_msgs = [(i, item[0]) for i, item in enumerate(pane.msg_data) if item[0].text]
        if num < 1 or num > len(text_msgs):
            self.notify(f"Message #{num} not found (1-{len(text_msgs)})", severity="warning")
            return

        idx, msg = text_msgs[num - 1]
        sender_name = pane.msg_data[idx][1]
        pane.reply_to_message = msg.id
        preview = msg.text[:50] if msg.text else "[Media]"
        if len(msg.text or "") > 50:
            preview += "..."
        pane.show_reply_preview(f"Reply to #{num} ({sender_name}): {preview}")
        self.notify(f"Replying to #{num}. Type your message and press Enter.", severity="info")
        inp = pane.get_input()
        if inp:
            inp.focus()

    def _handle_media_command(self, text: str, pane: ChatPane):
        """Handle /media N or /m N command to download and open media."""
        parts = text.split(maxsplit=1)
        if len(parts) < 2:
            self.notify("Usage: /media N or /m N", severity="warning")
            return
        try:
            num = int(parts[1])
        except ValueError:
            self.notify("Usage: /media N (where N is the message number)", severity="warning")
            return

        if not pane.msg_data:
            self.notify("No messages loaded", severity="warning")
            return

        if num < 1 or num > len(pane.msg_data):
            self.notify(f"Message #{num} not found (1-{len(pane.msg_data)})", severity="warning")
            return

        msg = pane.msg_data[num - 1][0]
        if not msg.media:
            self.notify(f"Message #{num} has no media", severity="warning")
            return

        self.notify(f"Downloading media from #{num}...", severity="info")
        self._download_and_open_media(msg, pane)

    def _download_and_open_media(self, msg, pane: ChatPane):
        """Download media and open it with system default application."""
        if not self.client or not self.telegram_loop:
            self.notify("Telegram connection not ready", severity="error")
            return

        import tempfile
        import subprocess
        import platform

        async def _download():
            try:
                # Create temp directory for downloads
                temp_dir = tempfile.gettempdir()
                
                # Download the media
                path = await self.client.download_media(msg, file=temp_dir)
                
                if path:
                    
                    # Open with system default application
                    def open_file():
                        try:
                            system = platform.system()
                            if system == "Darwin":  # macOS
                                subprocess.run(["open", path], check=True)
                            elif system == "Linux":
                                subprocess.run(["xdg-open", path], check=True)
                            elif system == "Windows":
                                subprocess.run(["start", "", path], shell=True, check=True)
                            self.notify(f"Opened: {os.path.basename(path)}", severity="success")
                        except Exception as e:
                            self.notify(f"Could not open file: {e}", severity="error")
                    
                    self.call_from_thread(open_file)
                else:
                    self.call_from_thread(lambda: self.notify("Download failed", severity="error"))
            except Exception as e:
                _log(f"Media download error: {e}", "ERROR")
                err_msg = str(e)
                self.call_from_thread(lambda err=err_msg: self.notify(f"Download error: {err}", severity="error"))

        asyncio.run_coroutine_threadsafe(_download(), self.telegram_loop)

    def _send_message(self, text: str, pane: ChatPane):
        if not self.client or not pane.chat_id:
            self.notify("Select a conversation first", severity="warning")
            return
        if not self.telegram_loop:
            self.notify("Telegram connection not ready", severity="error")
            return

        chat_id = pane.chat_id
        entity = self.chats[chat_id]['entity']
        reply_to_id = pane.reply_to_message

        async def _send():
            try:
                if reply_to_id:
                    reply_msg = None
                    if chat_id in self.messages:
                        for msg in self.messages[chat_id]:
                            if msg.id == reply_to_id:
                                reply_msg = msg
                                break
                    if reply_msg:
                        await self.client.send_message(entity, text, reply_to=reply_msg)
                    else:
                        await self.client.send_message(entity, text)
                else:
                    await self.client.send_message(entity, text)

                msg_data = await self._load_and_resolve(entity, chat_id)
                self.call_from_thread(lambda: self._display_messages_in_pane(chat_id, msg_data, pane))
            except Exception as e:
                _log(f"Send failed: {e}", "ERROR")

        asyncio.run_coroutine_threadsafe(_send(), self.telegram_loop)

    async def on_new_message(self, event) -> None:
        """Handle incoming new messages."""
        raw_chat_id = event.message.chat_id

        # Normalize chat_id - Telegram uses different formats:
        # Channels/supergroups: -100XXXXXXXXXX -> XXXXXXXXXX
        # Groups: -XXXXXXXXXX -> XXXXXXXXXX
        # Users: XXXXXXXXXX (positive)
        chat_id = raw_chat_id
        if raw_chat_id < 0:
            str_id = str(abs(raw_chat_id))
            if str_id.startswith("100"):
                normalized_id = int(str_id[3:])
                if normalized_id in self.chats:
                    chat_id = normalized_id
            if chat_id == raw_chat_id and abs(raw_chat_id) in self.chats:
                chat_id = abs(raw_chat_id)

        if chat_id not in self.chats:
            return

        # Find all panes showing this chat
        matching_panes = [p for p in self.panes if p.chat_id == chat_id]

        if matching_panes:
            try:
                entity = self.chats[chat_id]['entity']
                msg_data = await self._load_and_resolve(entity, chat_id)
                await self.client.send_read_acknowledge(entity)
                self.chats[chat_id]['unread'] = 0

                for pane in matching_panes:
                    def update_pane(p=pane, d=msg_data, cid=chat_id):
                        self._display_messages_in_pane(cid, d, p)
                    self.call_from_thread(update_pane)
            except Exception as e:
                _log(f"Failed to load in on_new_message: {e}", "ERROR")
        else:
            self.chats[chat_id]['unread'] = self.chats[chat_id].get('unread', 0) + 1
            chat_name = self.chats[chat_id].get('name', 'Unknown')
            preview = event.message.text[:30] if event.message.text else "[Media]"

            def _update_sidebar():
                try:
                    sidebar = self.query_one("#sidebar", ChatList)
                    sidebar.update_chats(self.chats)
                except Exception:
                    pass

            self.call_from_thread(_update_sidebar)
            self.call_from_thread(lambda: self.notify(f"{chat_name}: {preview}", severity="info"))

    def action_split_vertical(self) -> None:
        """Split the active pane vertically (side by side)."""
        self._do_split("horizontal")

    def action_split_horizontal(self) -> None:
        """Split the active pane horizontally (stacked)."""
        self._do_split("vertical")

    def _do_split(self, layout: str) -> None:
        """Split the active pane.

        Args:
            layout: Container layout - "horizontal" places panes side by side,
                    "vertical" stacks them on top of each other
        """
        if not self.active_pane:
            return

        old_pane = self.active_pane
        parent = old_pane.parent

        # Save current pane state before any DOM manipulation
        saved_chat_id = old_pane.chat_id
        saved_msg_data = old_pane.msg_data.copy() if old_pane.msg_data else []
        saved_reply_to = old_pane.reply_to_message

        # Check if parent is a SplitContainer with same layout - just add to it
        if isinstance(parent, SplitContainer) and parent.direction == layout:
            new_pane = ChatPane()
            self.panes.append(new_pane)
            parent.mount(new_pane, after=old_pane)
            self._set_active_pane(new_pane)
            return

        # Create split container and TWO new panes (can't reuse old_pane after remove)
        split = SplitContainer(direction=layout)
        pane1 = ChatPane()
        pane2 = ChatPane()

        # Mount split container where old pane is
        parent.mount(split, after=old_pane)

        # Remove old pane from parent and from our list
        self.panes.remove(old_pane)
        old_pane.remove()

        # Mount both new panes in split
        split.mount(pane1)
        split.mount(pane2)
        self.panes.append(pane1)
        self.panes.append(pane2)

        # Restore state to pane1 (the replacement for old_pane)
        pane1.chat_id = saved_chat_id
        pane1.msg_data = saved_msg_data
        pane1.reply_to_message = saved_reply_to

        # Redraw pane1 content after mount completes
        def restore_content(_delay=None):
            try:
                if saved_chat_id and saved_chat_id in self.chats:
                    chat_info = self.chats[saved_chat_id]
                    pane1.set_chat_header(chat_info['name'], chat_info.get('username', ''))
                    if saved_msg_data:
                        pane1.set_messages(self._format_messages(saved_msg_data, pane1))
                inp = pane2.get_input()
                if inp:
                    inp.focus()
            except Exception:
                pass

        self.call_later(restore_content, 0.15)

        # Set pane2 as active
        if self.active_pane:
            self.active_pane.remove_class("pane-focused")
        self.active_pane = pane2
        pane2.add_class("pane-focused")

    def action_close_pane(self):
        if len(self.panes) <= 1:
            self.notify("Cannot close the last pane", severity="warning")
            return
        if not self.active_pane:
            return

        pane = self.active_pane
        parent = pane.parent
        idx = self.panes.index(pane)
        self.panes.remove(pane)
        pane.remove()

        # If parent is a SplitContainer with only one child left, unwrap it
        if isinstance(parent, SplitContainer):
            remaining = list(parent.children)
            if len(remaining) == 1:
                grandparent = parent.parent
                child = remaining[0]
                child.remove()
                grandparent.mount(child, after=parent)
                parent.remove()
            elif len(remaining) == 0:
                parent.remove()

        new_idx = min(idx, len(self.panes) - 1)
        if self.panes:
            self._set_active_pane(self.panes[new_idx])

    def action_cycle_pane(self):
        if len(self.panes) <= 1:
            if self.active_pane:
                inp = self.active_pane.get_input()
                if inp:
                    inp.focus()
            return
        if not self.active_pane:
            self._set_active_pane(self.panes[0])
            return
        idx = self.panes.index(self.active_pane)
        next_idx = (idx + 1) % len(self.panes)
        self._set_active_pane(self.panes[next_idx])

    def action_refresh(self):
        if self.client:
            self.run_worker(self.load_conversations(), exclusive=False)

    def action_clear(self):
        if self.active_pane:
            self.active_pane.set_messages("")

    async def action_quit(self):
        self.save_layout()
        if self.client:
            await self.client.disconnect()
        self.exit()


# =============================================================================
# Entry Point
# =============================================================================

async def main() -> None:
    """Main entry point for the application."""
    config_file = "telegram_config.json"
    api_id = None
    api_hash = None

    if os.path.exists(config_file):
        try:
            with open(config_file, 'r') as f:
                config = json.load(f)
                api_id = config.get('api_id')
                api_hash = config.get('api_hash')
        except (json.JSONDecodeError, OSError):
            pass

    if not api_id or not api_hash:
        print("=" * 60)
        print(f"Telegram Terminal Client v{__version__} - First time setup")
        print("=" * 60)
        print("\nGet API credentials from: https://my.telegram.org")
        print("Log in and create a new application\n")
        try:
            api_id_str = input("API ID: ").strip()
            api_hash = input("API Hash: ").strip()
            api_id = int(api_id_str)
            with open(config_file, 'w') as f:
                json.dump({'api_id': api_id, 'api_hash': api_hash}, f)
        except (ValueError, KeyboardInterrupt, OSError):
            print("\nAborted.")
            return

    app = TelegramApp()
    app.api_id = api_id
    app.api_hash = api_hash
    app.save_config()
    await app.run_async()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass

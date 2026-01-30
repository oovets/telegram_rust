"""Widget classes for Telegram TUI client."""

from datetime import datetime
from typing import Dict, List, Optional

from textual.binding import Binding
from textual.containers import Horizontal, ScrollableContainer, Vertical
from textual.widgets import Input, ListItem, ListView, Static

from .messages import ChatSelected, PaneClicked
from .utils import _log


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
        self.filter_type: Optional[str] = None  # None, "sender", "media", "link"
        self.filter_value: Optional[str] = None  # sender name or media type
        
        # Performance optimization: cache formatted messages
        self._format_cache: Dict = {}  # (width, compact, emojis, reactions, timestamps, line_nums) -> formatted_text

    def compose(self):
        yield Static("", classes="pane-header")
        yield ScrollableContainer(
            Static("Select a conversation...", classes="pane-messages"),
            classes="pane-messages-container"
        )
        with Horizontal(classes="pane-input-container"):
            yield Static("", classes="pane-reply-preview")
            yield Input(placeholder="Type a message... (/reply N to reply)", classes="pane-input")

    def set_chat_header(self, name: str, username: str = "", pinned: str = None, online: str = "") -> None:
        """Update the pane header with chat name, online status, and optional pinned message."""
        header = self.query_one(".pane-header", Static)
        text = name
        if online:
            text += f" [{online}]"
        if username:
            text += f" {username}"
        if pinned:
            text += f" [dim]| Pinned: {pinned}[/dim]"
        self._header_base = text  # Store base header for typing indicator
        header.update(text)

    def show_typing_indicator(self, name: str) -> None:
        """Show typing indicator in header."""
        try:
            header = self.query_one(".pane-header", Static)
            base = getattr(self, '_header_base', '')
            header.update(f"{base} [italic dim]{name} is typing...[/italic dim]")
        except Exception:
            pass

    def hide_typing_indicator(self) -> None:
        """Hide typing indicator, restore original header."""
        try:
            header = self.query_one(".pane-header", Static)
            base = getattr(self, '_header_base', '')
            header.update(base)
        except Exception:
            pass

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

    def update_chats(self, chats: Dict[int, Dict], lazy: bool = True) -> None:
        """Update the chat list with new data.
        
        Args:
            chats: Dictionary of chat_id -> chat_info
            lazy: If True, only load first 30 chats initially (performance optimization)
        """
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
            
            # Lazy loading: only load first 30 chats initially for better performance
            load_count = 30 if lazy and len(sorted_chats) > 30 else len(sorted_chats)
            
            for chat_id, chat_info in sorted_chats[:load_count]:
                try:
                    item = ChatListItem(chat_id, chat_info)
                    self.chat_items[chat_id] = item
                    self.chat_list_view.append(item)
                except Exception:
                    pass

            self.chat_list_view.refresh()
            
            # If lazy loading, schedule remaining chats to load after a short delay
            if lazy and len(sorted_chats) > load_count:
                def load_remaining():
                    for chat_id, chat_info in sorted_chats[load_count:]:
                        try:
                            item = ChatListItem(chat_id, chat_info)
                            self.chat_items[chat_id] = item
                            self.chat_list_view.append(item)
                        except Exception:
                            pass
                    self.chat_list_view.refresh()
                
                # Load remaining after 500ms
                self.set_timer(0.5, load_remaining)
                
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

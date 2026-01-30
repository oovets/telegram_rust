"""Message classes for Telegram TUI client."""

from textual.message import Message
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .widgets import ChatPane


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

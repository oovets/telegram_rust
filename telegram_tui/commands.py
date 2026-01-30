"""Command handlers for Telegram TUI client.

This module contains all the /command handlers like /reply, /edit, /delete, etc.
"""

import asyncio
import os
import platform
import subprocess
import tempfile
from typing import TYPE_CHECKING

from telethon.tl.functions.channels import EditBannedRequest, InviteToChannelRequest
from telethon.tl.functions.messages import AddChatUserRequest, CreateChatRequest
from telethon.tl.types import Channel, Chat, ChatBannedRights, User

from .utils import _log

if TYPE_CHECKING:
    from .app import TelegramApp
    from .widgets import ChatPane


def handle_command(app: "TelegramApp", text: str, pane: "ChatPane") -> bool:
    """Handle a slash command. Returns True if handled."""
    
    if text.startswith("/reply "):
        app._handle_reply_command(text, pane)
        return True

    if text.startswith("/media ") or text.startswith("/m "):
        app._handle_media_command(text, pane)
        return True

    if text.startswith("/edit ") or text.startswith("/e "):
        app._handle_edit_command(text, pane)
        return True

    if text.startswith("/delete ") or text.startswith("/del ") or text.startswith("/d "):
        app._handle_delete_command(text, pane)
        return True

    if text.startswith("/alias "):
        app._handle_alias_command(text, pane)
        return True

    if text.startswith("/unalias "):
        app._handle_unalias_command(text, pane)
        return True

    if text.startswith("/filter"):
        app._handle_filter_command(text, pane)
        return True

    if text.startswith("/search ") or text.startswith("/s "):
        app._handle_search_command(text, pane)
        return True

    if text.startswith("/new "):
        app._handle_new_chat_command(text, pane)
        return True

    if text.startswith("/newgroup "):
        app._handle_new_group_command(text, pane)
        return True

    if text.startswith("/add "):
        app._handle_add_member_command(text, pane)
        return True

    if text.startswith("/kick ") or text.startswith("/remove "):
        app._handle_remove_member_command(text, pane)
        return True

    if text.startswith("/members"):
        app._handle_members_command(text, pane)
        return True

    if text.startswith("/forward ") or text.startswith("/fwd ") or text.startswith("/f "):
        app._handle_forward_command(text, pane)
        return True

    return False

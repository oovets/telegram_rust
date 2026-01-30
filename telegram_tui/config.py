"""Configuration management for Telegram TUI client."""

import json
import os
from typing import Dict, Optional

from .utils import _log


class Config:
    """Manages configuration, layout, and aliases."""

    def __init__(self, script_dir: str):
        self.script_dir = script_dir
        self.session_file = os.path.join(script_dir, "telegram_session.session")
        self.config_file = os.path.join(script_dir, "telegram_config.json")
        self.layout_file = os.path.join(script_dir, "telegram_layout.json")
        self.aliases_file = os.path.join(script_dir, "telegram_aliases.json")

        # Settings with defaults
        self.api_id: Optional[int] = None
        self.api_hash: Optional[str] = None
        self.show_reactions: bool = True
        self.desktop_notifications: bool = True
        self.compact_mode: bool = True
        self.show_emojis: bool = True
        self.show_line_numbers: bool = True
        self.show_timestamps: bool = True

        # Data
        self.aliases: Dict[int, str] = {}
        self.saved_layout: Optional[dict] = None

    def load_all(self) -> None:
        """Load all configuration files."""
        self.load_config()
        self.load_layout()
        self.load_aliases()

    def load_config(self) -> None:
        """Load main configuration from file."""
        if os.path.exists(self.config_file):
            try:
                with open(self.config_file, 'r') as f:
                    config = json.load(f)
                    self.api_id = config.get('api_id')
                    self.api_hash = config.get('api_hash')
                    self.show_reactions = config.get('show_reactions', True)
                    self.desktop_notifications = config.get('desktop_notifications', True)
                    self.compact_mode = config.get('compact_mode', True)
                    self.show_emojis = config.get('show_emojis', True)
                    self.show_line_numbers = config.get('show_line_numbers', True)
                    self.show_timestamps = config.get('show_timestamps', True)
            except Exception:
                pass

    def save_config(self) -> None:
        """Save main configuration to file."""
        try:
            with open(self.config_file, 'w') as f:
                json.dump({
                    'api_id': self.api_id,
                    'api_hash': self.api_hash,
                    'show_reactions': self.show_reactions,
                    'desktop_notifications': self.desktop_notifications,
                    'compact_mode': self.compact_mode,
                    'show_emojis': self.show_emojis,
                    'show_line_numbers': self.show_line_numbers,
                    'show_timestamps': self.show_timestamps,
                }, f)
        except Exception:
            pass

    def load_aliases(self) -> None:
        """Load user aliases from file."""
        if os.path.exists(self.aliases_file):
            try:
                with open(self.aliases_file, 'r') as f:
                    data = json.load(f)
                    # Convert string keys back to int
                    self.aliases = {int(k): v for k, v in data.items()}
            except (json.JSONDecodeError, OSError) as e:
                _log(f"Failed to load aliases: {e}", "ERROR")
                self.aliases = {}

    def save_aliases(self) -> None:
        """Save user aliases to file."""
        try:
            # Convert int keys to strings for JSON
            data = {str(k): v for k, v in self.aliases.items()}
            with open(self.aliases_file, 'w') as f:
                json.dump(data, f, indent=2)
        except (OSError, TypeError) as e:
            _log(f"Failed to save aliases: {e}", "ERROR")

    def load_layout(self) -> None:
        """Load saved layout from file."""
        if os.path.exists(self.layout_file):
            try:
                with open(self.layout_file, 'r') as f:
                    self.saved_layout = json.load(f)
            except (json.JSONDecodeError, OSError) as e:
                _log(f"Failed to load layout: {e}", "ERROR")
                self.saved_layout = None

    def save_layout(self, layout: dict) -> None:
        """Save layout to file."""
        try:
            with open(self.layout_file, 'w') as f:
                json.dump(layout, f, indent=2)
        except (OSError, TypeError) as e:
            _log(f"Failed to save layout: {e}", "ERROR")

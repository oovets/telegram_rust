#!/usr/bin/env python3
"""
Telegram Terminal Client - Entry point.

This is the main entry point that imports and runs the TelegramApp
from the telegram_tui package.
"""

import asyncio

from telegram_tui import TelegramApp, __version__
from telegram_tui.__main__ import main


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass

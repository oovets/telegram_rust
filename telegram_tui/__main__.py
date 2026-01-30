"""Entry point for running telegram_tui as a module."""

import asyncio
import json
import os

from .app import TelegramApp


async def main() -> None:
    """Main entry point for the application."""
    from . import __version__
    
    # Get project root directory (one level up from telegram_tui/)
    script_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    config_file = os.path.join(script_dir, "telegram_config.json")
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

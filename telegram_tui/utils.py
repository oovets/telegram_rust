"""Utility functions for Telegram TUI client."""

import os
import re
import urllib.parse
import urllib.request
from datetime import datetime

# Configuration
DEBUG_MODE = os.environ.get("TELEGRAM_DEBUG", "").lower() in ("1", "true", "yes")

try:
    _script_dir = os.path.dirname(os.path.abspath(__file__))
    _script_dir = os.path.dirname(_script_dir)  # Go up one level from telegram_tui/
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


def wrap_text(text: str, indent: int, width: int) -> str:
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


def shorten_url(url: str) -> str:
    """Shorten a URL using is.gd service."""
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


def shorten_urls_in_text(text: str) -> str:
    """Shorten long URLs in text."""
    url_pattern = re.compile(r'https?://\S{40,}')
    def replacer(match):
        url = match.group(0)
        if url not in _url_cache:
            _url_cache[url] = shorten_url(url)
        return _url_cache[url]
    return url_pattern.sub(replacer, text)


def strip_emojis(text: str) -> str:
    """Remove emojis from text."""
    emoji_pattern = re.compile(
        "["
        "\U0001F600-\U0001F64F"  # emoticons
        "\U0001F300-\U0001F5FF"  # symbols & pictographs
        "\U0001F680-\U0001F6FF"  # transport & map symbols
        "\U0001F1E0-\U0001F1FF"  # flags
        "\U00002702-\U000027B0"  # dingbats
        "\U000024C2-\U0001F251"  # enclosed characters
        "\U0001F900-\U0001F9FF"  # supplemental symbols
        "\U0001FA00-\U0001FA6F"  # chess symbols
        "\U0001FA70-\U0001FAFF"  # symbols extended-A
        "\U00002600-\U000026FF"  # misc symbols
        "\U00002700-\U000027BF"  # dingbats
        "\U0001F000-\U0001F02F"  # mahjong tiles
        "\U0001F0A0-\U0001F0FF"  # playing cards
        "]+",
        flags=re.UNICODE
    )
    return emoji_pattern.sub("", text)

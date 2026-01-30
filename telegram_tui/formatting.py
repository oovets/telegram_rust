"""Text formatting utilities for Telegram TUI client."""


def format_reactions(msg) -> str:
    """Format message reactions as a string."""
    if not hasattr(msg, 'reactions') or not msg.reactions:
        return ""

    try:
        results = msg.reactions.results
        if not results:
            return ""

        reaction_strs = []
        for r in results:
            count = r.count
            # Get the emoji or custom emoji
            if hasattr(r.reaction, 'emoticon'):
                emoji = r.reaction.emoticon
            elif hasattr(r.reaction, 'document_id'):
                emoji = "?"  # Custom emoji, can't display
            else:
                continue

            if count > 1:
                reaction_strs.append(f"{count}x{emoji}")
            else:
                reaction_strs.append(emoji)

        if reaction_strs:
            return " ".join(reaction_strs)
    except Exception:
        pass
    return ""


def get_media_label(msg) -> str:
    """Get a colored label for media attachments."""
    # Helper to get title from webpage
    def get_webpage_title(webpage, max_len=40):
        if hasattr(webpage, 'title') and webpage.title:
            title = webpage.title[:max_len].replace("[", "\\[")
            if len(webpage.title) > max_len:
                title += "..."
            return title
        return None

    # Check for webpage preview first (YouTube, Spotify, etc.)
    if hasattr(msg, 'media') and hasattr(msg.media, 'webpage') and msg.media.webpage:
        webpage = msg.media.webpage
        site_name = getattr(webpage, 'site_name', '') or ''
        url = getattr(webpage, 'url', '') or ''

        # YouTube
        if site_name == "YouTube" or "youtube.com" in url or "youtu.be" in url:
            title = get_webpage_title(webpage)
            if title:
                return f"[bold red]\\[YouTube: {title}][/bold red]"
            return "[bold red]\\[YouTube][/bold red]"

        # Spotify
        if site_name == "Spotify" or "spotify.com" in url:
            title = get_webpage_title(webpage)
            if title:
                return f"[bold green]\\[Spotify: {title}][/bold green]"
            return "[bold green]\\[Spotify][/bold green]"

    # Check text for links without preview
    if msg.text:
        if "youtube.com" in msg.text or "youtu.be" in msg.text:
            return "[bold red]\\[YouTube][/bold red]"
        if "open.spotify.com" in msg.text:
            return "[bold green]\\[Spotify][/bold green]"

    if msg.photo:
        return "[bold blue]\\[Photo][/bold blue]"
    elif msg.video:
        return "[bold magenta]\\[Video][/bold magenta]"
    elif msg.audio:
        return "[bold yellow]\\[Audio][/bold yellow]"
    elif msg.voice:
        return "[bold yellow]\\[Voice][/bold yellow]"
    elif msg.video_note:
        return "[bold magenta]\\[Video note][/bold magenta]"
    elif msg.sticker:
        # Try to get sticker emoji
        emoji = ""
        if hasattr(msg.sticker, 'attributes'):
            for attr in msg.sticker.attributes:
                if hasattr(attr, 'alt') and attr.alt:
                    emoji = f" {attr.alt}"
                    break
        return f"[bold cyan]\\[Sticker{emoji}][/bold cyan]"
    elif msg.gif:
        return "[bold magenta]\\[GIF][/bold magenta]"
    elif msg.document:
        name = ""
        if hasattr(msg.document, 'attributes'):
            for attr in msg.document.attributes:
                if hasattr(attr, 'file_name') and attr.file_name:
                    name = attr.file_name
                    break
        if name:
            safe_name = name.replace("[", "\\[")
            return f"[bold white]\\[Doc: {safe_name}][/bold white]"
        return "[bold white]\\[Document][/bold white]"
    elif msg.contact:
        contact_name = ""
        if hasattr(msg.contact, 'first_name'):
            contact_name = msg.contact.first_name
            if hasattr(msg.contact, 'last_name') and msg.contact.last_name:
                contact_name += f" {msg.contact.last_name}"
        if contact_name:
            safe_contact = contact_name.replace("[", "\\[")
            return f"[bold cyan]\\[Contact: {safe_contact}][/bold cyan]"
        return "[bold cyan]\\[Contact][/bold cyan]"
    elif msg.geo:
        return "[bold green]\\[Location][/bold green]"
    elif msg.poll:
        poll_question = ""
        if hasattr(msg.poll, 'poll') and hasattr(msg.poll.poll, 'question'):
            q = msg.poll.poll.question
            # Handle TextWithEntities object
            if hasattr(q, 'text'):
                q = q.text
            poll_question = str(q)[:30].replace("[", "\\[")
        if poll_question:
            return f"[bold yellow]\\[Poll: {poll_question}][/bold yellow]"
        return "[bold yellow]\\[Poll][/bold yellow]"
    return ""

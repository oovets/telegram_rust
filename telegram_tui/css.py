"""CSS styles for Telegram TUI client."""

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

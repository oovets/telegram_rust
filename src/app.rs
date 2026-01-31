use anyhow::Result;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::commands::CommandHandler;
use crate::config::Config;
use crate::formatting::format_messages_for_display;
use crate::persistence::{Aliases, AppState, LayoutData, PaneState};
use crate::split_view::{PaneNode, SplitDirection};
use crate::telegram::TelegramClient;
use crate::utils::{send_desktop_notification, try_autocomplete};
use crate::widgets::ChatPane;

pub struct App {
    pub config: Config,
    pub telegram: TelegramClient,
    pub my_user_id: i64,  // Current user's ID for determining outgoing messages
    pub chats: Vec<ChatInfo>,
    pub selected_chat_idx: usize,
    pub panes: Vec<ChatPane>,
    pub focused_pane_idx: usize,
    pub pane_tree: PaneNode,
    pub input_history: Vec<String>,
    pub history_idx: Option<usize>,
    pub history_temp: String, // Save current input when browsing history
    pub aliases: Aliases,
    pub focus_on_chat_list: bool,
    pub status_message: Option<String>, // Notification bar at bottom
    pub status_expire: Option<std::time::Instant>,
    pub pane_areas: std::collections::HashMap<usize, Rect>, // Track pane screen positions
    pub chat_list_area: Option<Rect>, // Track chat list area for mouse clicks

    // Settings
    pub show_reactions: bool,
    pub show_notifications: bool,
    pub compact_mode: bool,
    pub show_emojis: bool,
    pub show_line_numbers: bool,
    pub show_timestamps: bool,
    pub show_chat_list: bool,
}

#[derive(Clone)]
pub struct ChatInfo {
    pub id: i64,
    pub name: String,
    pub username: Option<String>,
    pub unread: u32,
    pub is_channel: bool,
    pub is_group: bool,
}

impl App {
    pub async fn new() -> Result<Self> {
        let config = Config::load()?;
        let telegram = TelegramClient::new(&config).await?;
        let my_user_id = telegram.get_me().await?;
        let app_state = AppState::load(&config).unwrap_or_else(|_| AppState {
            settings: crate::persistence::AppSettings::default(),
            aliases: Aliases::default(),
            layout: LayoutData::default(),
        });

        // Load initial chats
        let chats = telegram.get_dialogs().await.unwrap_or_else(|_| Vec::new());

        Ok(Self {
            config,
            telegram,
            my_user_id,
            chats,
            selected_chat_idx: 0,
            panes: vec![ChatPane::new()],
            focused_pane_idx: 0,
            pane_tree: PaneNode::new_single(0),
            input_history: Vec::new(),
            history_idx: None,
            history_temp: String::new(),
            aliases: app_state.aliases,
            focus_on_chat_list: true,
            status_message: None,
            status_expire: None,
            chat_list_area: None,
            pane_areas: std::collections::HashMap::new(),
            show_reactions: app_state.settings.show_reactions,
            show_notifications: app_state.settings.show_notifications,
            compact_mode: app_state.settings.compact_mode,
            show_emojis: app_state.settings.show_emojis,
            show_line_numbers: app_state.settings.show_line_numbers,
            show_timestamps: app_state.settings.show_timestamps,
            show_chat_list: true,
        })
    }

    pub fn draw(&mut self, f: &mut Frame) {
        // Check typing indicators for expiry
        for pane in &mut self.panes {
            pane.check_typing_expired();
        }
        // Check status message expiry
        if let Some(expire) = self.status_expire {
            if std::time::Instant::now() >= expire {
                self.status_message = None;
                self.status_expire = None;
            }
        }

        let has_status = self.status_message.is_some();
        let main_constraints = if has_status {
            vec![Constraint::Min(0), Constraint::Length(1)]
        } else {
            vec![Constraint::Min(0)]
        };

        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints(main_constraints)
            .split(f.area());

        let (chat_area, pane_area) = if self.show_chat_list {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(20), Constraint::Percentage(80)])
                .split(outer[0]);
            (Some(chunks[0]), chunks[1])
        } else {
            (None, outer[0])
        };

        // Store chat list area for mouse handling
        if let Some(area) = chat_area {
            self.chat_list_area = Some(area);
            self.draw_chat_list(f, area);
        } else {
            self.chat_list_area = None;
        }

        let render_fn = |f: &mut Frame, area: Rect, pane: &ChatPane, is_focused: bool| {
            self.draw_chat_pane_impl(f, area, pane, is_focused);
        };

        let mut pane_areas = std::collections::HashMap::new();
        self.pane_tree
            .render(f, pane_area, &self.panes, self.focused_pane_idx, &render_fn, &mut pane_areas);
        self.pane_areas = pane_areas;

        // Draw status bar
        if has_status {
            if let Some(ref msg) = self.status_message {
                let status = Paragraph::new(msg.as_str())
                    .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
                f.render_widget(status, outer[1]);
            }
        }
    }

    fn draw_chat_list(&self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .chats
            .iter()
            .enumerate()
            .map(|(idx, chat)| {
                let style = if idx == self.selected_chat_idx {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let unread_str = if chat.unread > 0 {
                    format!(" ({})", chat.unread)
                } else {
                    String::new()
                };

                // Build full content first
                let mut content = format!("{}{}", chat.name, unread_str);
                if let Some(ref username) = chat.username {
                    if !username.is_empty() {
                        content.push_str(&format!(" {}", username));
                    }
                }

                // Then truncate if needed
                let max_width = area.width.saturating_sub(6).max(1) as usize;
                let char_count = content.chars().count();
                if char_count > max_width && max_width > 0 {
                    let nth_index = max_width.saturating_sub(1);
                    if let Some((truncate_at, _)) = content.char_indices().nth(nth_index) {
                        content.truncate(truncate_at);
                        content.push('…');
                    }
                }

                ListItem::new(content).style(style)
            })
            .collect();

        let border_style = if self.focus_on_chat_list {
            Style::default().fg(Color::Green)
        } else {
            Style::default()
        };

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Chats")
                    .border_style(border_style),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        f.render_widget(list, area);
    }

    fn draw_chat_pane_impl(
        &self,
        f: &mut Frame,
        area: Rect,
        pane: &ChatPane,
        is_focused: bool,
    ) {
        let has_reply_preview = pane.reply_preview.is_some();
        let constraints = if has_reply_preview {
            vec![
                Constraint::Length(3),  // header
                Constraint::Min(0),     // messages
                Constraint::Length(1),  // reply preview
                Constraint::Length(3),  // input
            ]
        } else {
            vec![
                Constraint::Length(3),  // header
                Constraint::Min(0),     // messages
                Constraint::Length(3),  // input
            ]
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        // Header with online status, username, pinned, typing
        let header_style = if is_focused {
            if self.focus_on_chat_list {
                // Show which pane will receive the next chat from list
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                // Active input pane
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            }
        } else {
            Style::default().fg(Color::Cyan)
        };

        let mut header_text = String::new();
        if is_focused && self.focus_on_chat_list {
            header_text.push_str("[TARGET] ");
        }
        header_text.push_str(&pane.header_text());
        
        let header = Paragraph::new(header_text)
            .block(Block::default().borders(Borders::ALL))
            .style(header_style);
        f.render_widget(header, chunks[0]);

        // Messages - use rich formatted data if available, otherwise plain messages
        let message_width = chunks[1].width.saturating_sub(4) as usize;
        let display_lines = if !pane.msg_data.is_empty() {
            // Use msg_data for rich formatting
            let filter_type = pane
                .filter_type
                .as_ref()
                .map(|ft| match ft {
                    crate::widgets::FilterType::Sender => "sender",
                    crate::widgets::FilterType::Media => "media",
                    crate::widgets::FilterType::Link => "link",
                });
            let filter_value = pane.filter_value.as_deref();

            let mut lines = format_messages_for_display(
                &pane.msg_data,
                message_width,
                self.compact_mode,
                self.show_emojis,
                self.show_reactions,
                self.show_timestamps,
                self.show_line_numbers,
                filter_type,
                filter_value,
                pane.unread_count_at_load,
                &self.aliases.map,
            );
            
            // Append any status messages from pane.messages (like "✓ Replied to #5")
            if !pane.messages.is_empty() {
                lines.push(String::new()); // Separator
                lines.extend(pane.messages.clone());
            }
            lines
        } else {
            // Fallback to plain messages (for status messages, etc.)
            pane.messages.clone()
        };

        let message_lines: Vec<Line> = display_lines
            .iter()
            .flat_map(|msg| {
                if msg.is_empty() {
                    return vec![Line::from("")];
                }
                
                // Determine styling based on message content
                let (display_msg, style) = if msg.starts_with("[REPLY_TO_ME]") {
                    // Reply to my own message: remove marker and style in red
                    let clean_msg = msg.replace("[REPLY_TO_ME]", "").trim_start().to_string();
                    (clean_msg, Style::default().fg(Color::Red).add_modifier(Modifier::ITALIC))
                } else if msg.starts_with("  ↳ Reply to") {
                    // Other reply lines: gray and italic
                    (msg.to_string(), Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC))
                } else if msg.contains("[OUT]") {
                    // Outgoing message: remove marker and style name in green
                    let clean_msg = msg.replace("[OUT]", "");
                    (clean_msg, Style::default().fg(Color::Green))
                } else if msg.contains("[IN]") {
                    // Incoming message: remove marker and style name in cyan
                    let clean_msg = msg.replace("[IN]", "");
                    (clean_msg, Style::default().fg(Color::Cyan))
                } else {
                    // Default style
                    (msg.to_string(), Style::default())
                };
                
                let max_width = message_width;
                if display_msg.len() > max_width && max_width > 0 {
                    let mut lines = Vec::new();
                    let mut current_line = String::new();

                    for word in display_msg.split_whitespace() {
                        if current_line.len() + word.len() + 1 > max_width {
                            if !current_line.is_empty() {
                                lines.push(Line::from(current_line.clone()).style(style));
                                current_line.clear();
                            }
                            if word.chars().count() > max_width {
                                let split_at = word
                                    .char_indices()
                                    .nth(max_width)
                                    .map(|(i, _)| i)
                                    .unwrap_or(word.len());
                                lines.push(Line::from(word[..split_at].to_string()).style(style));
                                current_line = word[split_at..].to_string();
                            } else {
                                current_line = word.to_string();
                            }
                        } else {
                            if !current_line.is_empty() {
                                current_line.push(' ');
                            }
                            current_line.push_str(word);
                        }
                    }
                    if !current_line.is_empty() {
                        lines.push(Line::from(current_line).style(style));
                    }
                    lines
                } else {
                    vec![Line::from(display_msg).style(style)]
                }
            })
            .collect();

        // Calculate scroll position - we want to show the latest messages (at the bottom)
        let available_height = chunks[1].height.saturating_sub(2) as usize; // -2 for borders
        let total_lines = message_lines.len();
        
        // Auto-scroll to bottom if scroll_offset is 0 (default/new chat)
        let actual_scroll = if pane.scroll_offset == 0 && total_lines > available_height {
            total_lines.saturating_sub(available_height)
        } else {
            pane.scroll_offset
        };

        let messages = Paragraph::new(message_lines)
            .block(Block::default().borders(Borders::ALL).title("Messages"))
            .scroll((actual_scroll as u16, 0));
        f.render_widget(messages, chunks[1]);

        // Reply preview bar (if active)
        if has_reply_preview {
            if let Some(ref preview) = pane.reply_preview {
                let reply_bar = Paragraph::new(preview.as_str())
                    .style(Style::default().fg(Color::Magenta).add_modifier(Modifier::ITALIC));
                f.render_widget(reply_bar, chunks[2]);
            }
        }

        // Input
        let input_chunk = if has_reply_preview { chunks[3] } else { chunks[2] };
        let input_title = if is_focused && !self.focus_on_chat_list {
            "Input (type message, /command, or Tab to cycle)"
        } else {
            "Input"
        };
        let input_text = if is_focused { &pane.input_buffer } else { "" };
        let input = Paragraph::new(input_text)
            .block(Block::default().borders(Borders::ALL).title(input_title));
        f.render_widget(input, input_chunk);
    }

    pub async fn refresh_chats(&mut self) -> Result<()> {
        self.chats = self.telegram.get_dialogs().await?;
        self.notify("Chats refreshed");
        Ok(())
    }

    /// Show a status notification that auto-expires
    pub fn notify(&mut self, message: &str) {
        self.status_message = Some(message.to_string());
        self.status_expire =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
    }

    // =========================================================================
    // Split pane management
    // =========================================================================

    pub fn split_vertical(&mut self) {
        let new_pane = ChatPane::new();
        let new_idx = self.panes.len();
        self.panes.push(new_pane);

        self.split_pane_in_tree(self.focused_pane_idx, SplitDirection::Vertical, new_idx);
        self.focused_pane_idx = new_idx;
        self.focus_on_chat_list = false;
    }

    pub fn split_horizontal(&mut self) {
        let new_pane = ChatPane::new();
        let new_idx = self.panes.len();
        self.panes.push(new_pane);

        self.split_pane_in_tree(self.focused_pane_idx, SplitDirection::Horizontal, new_idx);
        self.focused_pane_idx = new_idx;
        self.focus_on_chat_list = false;
    }

    fn split_pane_in_tree(
        &mut self,
        target_idx: usize,
        direction: SplitDirection,
        new_idx: usize,
    ) {
        Self::split_node_recursive_static(&mut self.pane_tree, target_idx, direction, new_idx);
    }

    fn split_node_recursive_static(
        node: &mut PaneNode,
        target_idx: usize,
        direction: SplitDirection,
        new_idx: usize,
    ) -> bool {
        match node {
            PaneNode::Single(idx) if *idx == target_idx => {
                node.split(direction, new_idx);
                true
            }
            PaneNode::Split { children, .. } => {
                for child in children.iter_mut() {
                    if Self::split_node_recursive_static(child, target_idx, direction, new_idx) {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    pub fn close_pane(&mut self) {
        let pane_count_before = self.pane_tree.count_panes();
        if pane_count_before <= 1 {
            self.notify("Cannot close the last pane");
            return;
        }
        
        let focused_idx = self.focused_pane_idx;
        let removed = self.pane_tree.find_and_remove_pane(focused_idx);
        
        if removed {
            let pane_count_after = self.pane_tree.count_panes();
            self.notify(&format!("Closed pane {} ({} -> {} panes)", focused_idx, pane_count_before, pane_count_after));
            
            let remaining = self.pane_tree.get_pane_indices();
            if !remaining.is_empty() {
                self.focused_pane_idx = remaining[0];
            }
        } else {
            self.notify("Failed to close pane");
        }
    }

    pub fn clear_pane(&mut self) {
        if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
            pane.clear();
        }
    }

    pub fn cycle_focus(&mut self) {
        let all_panes = self.pane_tree.get_pane_indices();
        
        if all_panes.is_empty() {
            return;
        }

        if self.focus_on_chat_list {
            // Going from chat list to first pane
            self.focus_on_chat_list = false;
            self.focused_pane_idx = all_panes[0];
            self.notify(&format!("Focus: Pane #{}", self.focused_pane_idx + 1));
        } else {
            // Find current pane position
            if let Some(current_pos) = all_panes.iter().position(|&idx| idx == self.focused_pane_idx) {
                if current_pos + 1 < all_panes.len() {
                    // Go to next pane
                    self.focused_pane_idx = all_panes[current_pos + 1];
                    self.notify(&format!("Focus: Pane #{}", self.focused_pane_idx + 1));
                } else {
                    // Last pane, go back to chat list
                    self.focus_on_chat_list = true;
                    self.notify("Focus: Chat List");
                }
            } else {
                // Current pane not found, reset to first
                self.focused_pane_idx = all_panes[0];
                self.notify(&format!("Focus: Pane #{}", self.focused_pane_idx + 1));
            }
        }
    }

    // =========================================================================
    // Toggle settings (matching Python's action_toggle_*)
    // =========================================================================

    pub fn toggle_reactions(&mut self) {
        self.show_reactions = !self.show_reactions;
        let status = if self.show_reactions { "ON" } else { "OFF" };
        self.notify(&format!("Reactions: {}", status));
        self.refresh_all_pane_displays();
    }

    pub fn toggle_notifications(&mut self) {
        self.show_notifications = !self.show_notifications;
        let status = if self.show_notifications {
            "ON"
        } else {
            "OFF"
        };
        self.notify(&format!("Desktop notifications: {}", status));
    }

    pub fn toggle_compact(&mut self) {
        self.compact_mode = !self.compact_mode;
        let status = if self.compact_mode { "ON" } else { "OFF" };
        self.notify(&format!("Compact mode: {}", status));
        self.refresh_all_pane_displays();
    }

    pub fn toggle_emojis(&mut self) {
        self.show_emojis = !self.show_emojis;
        let status = if self.show_emojis { "ON" } else { "OFF" };
        self.notify(&format!("Emojis: {}", status));
        self.refresh_all_pane_displays();
    }

    pub fn toggle_line_numbers(&mut self) {
        self.show_line_numbers = !self.show_line_numbers;
        let status = if self.show_line_numbers { "ON" } else { "OFF" };
        self.notify(&format!("Line numbers: {}", status));
        self.refresh_all_pane_displays();
    }

    pub fn toggle_timestamps(&mut self) {
        self.show_timestamps = !self.show_timestamps;
        let status = if self.show_timestamps { "ON" } else { "OFF" };
        self.notify(&format!("Timestamps: {}", status));
        self.refresh_all_pane_displays();
    }

    pub fn toggle_chat_list(&mut self) {
        self.show_chat_list = !self.show_chat_list;
        self.notify(&format!("Chat list: {}", if self.show_chat_list { "ON" } else { "OFF" }));
    }

    /// Handle mouse click to select pane or open chat
    pub fn handle_mouse_click(&mut self, x: u16, y: u16) {
        // Check if clicking on a pane
        for (&pane_idx, &area) in &self.pane_areas {
            if x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height {
                // Clicked on this pane - make it active
                self.focused_pane_idx = pane_idx;
                self.focus_on_chat_list = false;
                self.notify(&format!("Active: Pane #{}", pane_idx + 1));
                return;
            }
        }
    }

    /// Handle mouse click on chat list
    pub async fn handle_chat_list_click(&mut self, y: u16, list_area: Rect) -> Result<()> {
        // Calculate which chat was clicked based on Y position
        // Each chat item is 1 line, starting at list_area.y + 1 (after top border)
        if y < list_area.y + 1 || y >= list_area.y + list_area.height - 1 {
            return Ok(()); // Clicked on border
        }
        
        let relative_y = (y - list_area.y - 1) as usize;
        if relative_y < self.chats.len() {
            // Open this chat in the focused pane
            if let Some(chat) = self.chats.get(relative_y) {
                let chat_id = chat.id;
                let chat_name = chat.name.clone();
                let chat_username = chat.username.clone();
                let raw_messages = self.telegram.get_messages(chat_id, 50).await?;

                let msg_data: Vec<crate::widgets::MessageData> = raw_messages
                    .iter()
                    .map(|(msg_id, sender_id, sender_name, text, reply_to_id, media_type, reactions)| {
                        let reply_to_msg_id = *reply_to_id;
                        
                        crate::widgets::MessageData {
                            msg_id: *msg_id,
                            sender_id: *sender_id,
                            sender_name: sender_name.clone(),
                            text: text.clone(),
                            is_outgoing: *sender_id == self.my_user_id,
                            timestamp: chrono::Utc::now().timestamp(),
                            media_type: media_type.clone(),
                            media_label: None,
                            reactions: reactions.clone(),
                            reply_to_msg_id,
                            reply_sender: None,
                            reply_text: None,
                        }
                    })
                    .collect();

                if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
                    pane.chat_id = Some(chat_id);
                    pane.chat_name = chat_name;
                    pane.username = chat_username;
                    pane.msg_data = msg_data;
                    pane.reply_to_message = None;
                    pane.hide_reply_preview();
                    pane.scroll_offset = 0;

                    if let Some(chat_info) = self.chats.iter_mut().find(|c| c.id == chat_id) {
                        chat_info.unread = 0;
                    }
                }
            }
        }
        Ok(())
    }

    /// Refresh all pane message displays (after toggling display settings)
    fn refresh_all_pane_displays(&mut self) {
        // Clear format caches so they re-render with new settings
        for pane in &mut self.panes {
            pane.format_cache.clear();
        }
    }

    // =========================================================================
    // Input handling
    // =========================================================================

    pub fn handle_up(&mut self) {
        if self.focus_on_chat_list {
            if self.selected_chat_idx > 0 {
                self.selected_chat_idx -= 1;
            }
        } else {
            // Browse input history
            if !self.input_history.is_empty() {
                if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
                    match self.history_idx {
                        None => {
                            // Save current input and start browsing
                            self.history_temp = pane.input_buffer.clone();
                            self.history_idx = Some(self.input_history.len() - 1);
                            pane.input_buffer = self.input_history[self.input_history.len() - 1].clone();
                    }
                        Some(idx) if idx > 0 => {
                            self.history_idx = Some(idx - 1);
                            pane.input_buffer = self.input_history[idx - 1].clone();
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    pub fn handle_down(&mut self) {
        if self.focus_on_chat_list {
            if self.selected_chat_idx < self.chats.len().saturating_sub(1) {
                self.selected_chat_idx += 1;
            }
        } else {
            // Browse input history
            if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
                if let Some(idx) = self.history_idx {
                    if idx + 1 < self.input_history.len() {
                        self.history_idx = Some(idx + 1);
                        pane.input_buffer = self.input_history[idx + 1].clone();
                    } else {
                        // Back to current input
                        self.history_idx = None;
                        pane.input_buffer = self.history_temp.clone();
                    }
                }
            }
        }
    }

    pub fn handle_page_up(&mut self) {
        if !self.focus_on_chat_list {
            if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
                pane.scroll_up();
            }
        }
    }

    pub fn handle_page_down(&mut self) {
        if !self.focus_on_chat_list {
            if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
                pane.scroll_down();
            }
        }
    }

    /// Handle Tab key: try autocomplete first, then cycle focus
    pub fn handle_tab(&mut self) {
        let is_empty = self.panes.get(self.focused_pane_idx)
            .map_or(true, |p| p.input_buffer.is_empty());
        
        if is_empty {
            self.cycle_focus();
            return;
        }

        // Try autocomplete
        if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
            let (completed, hint) = try_autocomplete(&pane.input_buffer);
            if let Some(completed) = completed {
                pane.input_buffer = completed;
            } else if let Some(hint) = hint {
                self.notify(&hint);
            } else {
                self.cycle_focus();
            }
        }
    }

    pub async fn handle_enter(&mut self) -> Result<()> {
        let input_empty = self.panes.get(self.focused_pane_idx)
            .map_or(true, |p| p.input_buffer.is_empty());
        
        if input_empty {
            if self.focus_on_chat_list && !self.chats.is_empty() {
                if let Some(chat) = self.chats.get(self.selected_chat_idx) {
                    let chat_id = chat.id;
                    let chat_name = chat.name.clone();
                    let chat_username = chat.username.clone();
                    let raw_messages = self.telegram.get_messages(chat_id, 50).await?;

                    // Convert to MessageData for proper formatting support
                    let msg_data: Vec<crate::widgets::MessageData> = raw_messages
                        .iter()
                        .map(|(msg_id, sender_id, sender_name, text, reply_to_id, media_type, reactions)| {
                            let reply_to_msg_id = *reply_to_id;
                            
                            crate::widgets::MessageData {
                                msg_id: *msg_id,
                                sender_id: *sender_id,
                                sender_name: sender_name.clone(),
                                text: text.clone(),
                                is_outgoing: *sender_id == self.my_user_id,
                                timestamp: chrono::Utc::now().timestamp(),
                                media_type: media_type.clone(),
                                media_label: None,
                                reactions: reactions.clone(),
                                reply_to_msg_id,
                                reply_sender: None,
                                reply_text: None,
                            }
                        })
                        .collect();

                    if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
                        
                        pane.chat_id = Some(chat_id);
                        pane.chat_name = chat_name;
                        pane.username = chat_username;
                        pane.msg_data = msg_data;
                        // Don't clear messages - they may contain status messages
                        pane.reply_to_message = None;
                        pane.hide_reply_preview();
                        // Don't set scroll_offset yet - let it be calculated during render
                        pane.scroll_offset = 0;

                        // Mark chat as read
                        if let Some(chat_info) =
                            self.chats.iter_mut().find(|c| c.id == chat_id)
                        {
                            pane.unread_count_at_load = chat_info.unread;
                            chat_info.unread = 0;
                        }
                    }
                    self.focus_on_chat_list = false;
                }
            }
        } else if !self.focus_on_chat_list {
            // Get input from active pane
            let (input_text, chat_id, reply_to_id) = if let Some(pane) = self.panes.get(self.focused_pane_idx) {
                (pane.input_buffer.clone(), pane.chat_id, pane.reply_to_message)
            } else {
                return Ok(());
            };

            // Save to history (no duplicates)
            if self.input_history.last().map_or(true, |last| last != &input_text) {
                self.input_history.push(input_text.clone());
                if self.input_history.len() > 100 {
                    self.input_history.remove(0);
                }
            }
            self.history_idx = None;
            self.history_temp.clear();

            // Try command handling
            if input_text.starts_with('/') {
                let focused = self.focused_pane_idx;
                let handled = CommandHandler::handle(self, &input_text, focused).await?;
                if handled {
                    if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
                        pane.input_buffer.clear();
                    }
                    return Ok(());
                }
            }

            // Handle reply mode or normal send
            if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
                if let (Some(chat_id), Some(reply_to_id)) =
                    (pane.chat_id, pane.reply_to_message)
                {
                    self.telegram
                        .reply_to_message(chat_id, reply_to_id, &input_text)
                        .await?;
                    pane.reply_to_message = None;
                    pane.hide_reply_preview();
                    pane.add_message(format!("✓ Replied to message ID {}", reply_to_id));
                } else if let Some(chat_id) = pane.chat_id {
                    self.telegram
                        .send_message(chat_id, &input_text)
                        .await?;
                }
                pane.input_buffer.clear();
            }
        }
        Ok(())
    }

    pub fn handle_char(&mut self, c: char) {
        if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
            pane.input_buffer.push(c);
        }
        self.history_idx = None;
    }

    pub fn handle_backspace(&mut self) {
        if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
            pane.input_buffer.pop();
        }
        self.history_idx = None;
    }

    // =========================================================================
    // New message handling
    // =========================================================================

    pub async fn process_telegram_events(&mut self) -> Result<()> {
        // Process incoming updates
        let updates = self.telegram.poll_updates().await?;

        for update in updates {
            match update {
                crate::telegram::TelegramUpdate::NewMessage {
                    chat_id,
                    sender_name: _,
                    text,
                    is_outgoing,
                } => {
                    // Normalize chat_id
                    let normalized_id = crate::utils::normalize_chat_id(chat_id);

                    // Check if any pane has this chat open
                    let matching_panes: Vec<usize> = self
                        .panes
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| {
                            p.chat_id == Some(chat_id) || p.chat_id == Some(normalized_id)
                        })
                        .map(|(i, _)| i)
                        .collect();

                    if !matching_panes.is_empty() {
                        // Reload messages for matching panes
                        let target_id = if self
                            .panes
                            .iter()
                            .any(|p| p.chat_id == Some(chat_id))
                        {
                            chat_id
                        } else {
                            normalized_id
                        };

                        if let Ok(raw_messages) =
                            self.telegram.get_messages(target_id, 50).await
                        {
                            // Convert to MessageData for proper formatting support
                            let msg_data: Vec<crate::widgets::MessageData> = raw_messages
                                .iter()
                                .map(|(msg_id, sender_id, sender_name, text, reply_to_id, media_type, reactions)| {
                                    let reply_to_msg_id = *reply_to_id;
                                    
                                    crate::widgets::MessageData {
                                        msg_id: *msg_id,
                                        sender_id: *sender_id,
                                        sender_name: sender_name.clone(),
                                        text: text.clone(),
                                        is_outgoing: *sender_id == self.my_user_id,
                                        timestamp: chrono::Utc::now().timestamp(),
                                        media_type: media_type.clone(),
                                        media_label: None,
                                        reactions: reactions.clone(),
                                        reply_to_msg_id,
                                        reply_sender: None,
                                        reply_text: None,
                                    }
                                })
                                .collect();

                            for idx in &matching_panes {
                                if let Some(pane) = self.panes.get_mut(*idx) {
                                    pane.msg_data = msg_data.clone();
                                    // Don't clear messages - they may contain status messages
                                }
                            }
                        }
                    } else {
                        // Increment unread for chats not in view
                        if let Some(chat_info) = self
                            .chats
                            .iter_mut()
                            .find(|c| c.id == chat_id || c.id == normalized_id)
                        {
                            chat_info.unread += 1;
                            let chat_name = chat_info.name.clone();
                            let preview = if text.chars().count() > 50 {
                                let truncate_at = text
                                    .char_indices()
                                    .nth(50)
                                    .map(|(i, _)| i)
                                    .unwrap_or(text.len());
                                format!("{}...", &text[..truncate_at])
                            } else {
                                text.clone()
                            };

                            // Desktop notification
                            if self.show_notifications && !is_outgoing {
                                send_desktop_notification(&chat_name, &preview);
                            }

                            self.notify(&format!("{}: {}", chat_name, preview));
                        }
                    }
                }
                crate::telegram::TelegramUpdate::UserTyping {
                    chat_id,
                    user_name,
                } => {
                    let normalized_id = crate::utils::normalize_chat_id(chat_id);
                    for pane in &mut self.panes {
                        if pane.chat_id == Some(chat_id)
                            || pane.chat_id == Some(normalized_id)
                        {
                            pane.show_typing_indicator(&user_name);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    // =========================================================================
    // State persistence
    // =========================================================================

    pub fn save_state(&self) -> Result<()> {
        let layout = LayoutData {
            panes: self
                .panes
                .iter()
                .map(|p| PaneState {
                    chat_id: p.chat_id,
                    chat_name: p.chat_name.clone(),
                    scroll_offset: p.scroll_offset,
                })
                .collect(),
            focused_pane: self.focused_pane_idx,
        };
        layout.save(&self.config)?;

        self.aliases.save(&self.config)?;

        let mut config = self.config.clone();
        config.settings.show_reactions = self.show_reactions;
        config.settings.show_notifications = self.show_notifications;
        config.settings.compact_mode = self.compact_mode;
        config.settings.show_emojis = self.show_emojis;
        config.settings.show_line_numbers = self.show_line_numbers;
        config.settings.show_timestamps = self.show_timestamps;
        config.save()?;

        Ok(())
    }
}

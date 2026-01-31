use anyhow::Result;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, Padding, Paragraph, Wrap},
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
    pub needs_redraw: bool,

    // Settings
    pub show_reactions: bool,
    pub show_notifications: bool,
    pub compact_mode: bool,
    pub show_emojis: bool,
    pub show_line_numbers: bool,
    pub show_timestamps: bool,
    pub show_chat_list: bool,
    pub show_user_colors: bool,
    pub show_borders: bool,
    pub user_colors: std::collections::HashMap<i64, Color>, // Map sender_id to color for group chats
}

#[derive(Clone)]
pub struct ChatInfo {
    pub id: i64,
    pub name: String,
    pub username: Option<String>,
    pub unread: u32,
    pub _is_channel: bool,
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

        // Load pane tree first to know which panes we need
        let (pane_tree, required_indices) = if let Some(saved_tree) = app_state.layout.pane_tree {
            let indices = saved_tree.get_pane_indices();
            (saved_tree, indices)
        } else {
            // No saved tree, create default based on number of saved panes
            let tree = if !app_state.layout.panes.is_empty() && app_state.layout.panes.len() > 1 {
                let mut t = PaneNode::new_single(0);
                for i in 1..app_state.layout.panes.len() {
                    t.split(SplitDirection::Vertical, i);
                }
                t
            } else {
                PaneNode::new_single(0)
            };
            let indices = tree.get_pane_indices();
            (tree, indices)
        };
        
        // Determine how many panes we need (max of what tree references and what's saved)
        let max_required_idx = required_indices.iter().max().copied().unwrap_or(0);
        let total_panes_needed = (max_required_idx + 1).max(app_state.layout.panes.len()).max(1);
        
        // Load panes - create panes for all indices up to total_panes_needed
        let mut panes: Vec<ChatPane> = Vec::new();
        for i in 0..total_panes_needed {
            if let Some(ps) = app_state.layout.panes.get(i) {
                // Load saved pane state
                let mut pane = ChatPane::new();
                pane.chat_id = ps.chat_id;
                pane.chat_name = ps.chat_name.clone();
                pane.scroll_offset = ps.scroll_offset;
                // Load filter settings
                if let Some(ref filter_type_str) = ps.filter_type {
                    pane.filter_type = Some(match filter_type_str.as_str() {
                        "sender" => crate::widgets::FilterType::Sender,
                        "media" => crate::widgets::FilterType::Media,
                        "link" => crate::widgets::FilterType::Link,
                        _ => {
                            panes.push(pane);
                            continue;
                        }
                    });
                }
                pane.filter_value = ps.filter_value.clone();
                panes.push(pane);
            } else {
                // Create empty pane for missing index
                panes.push(ChatPane::new());
            }
        }
        
        let focused_pane_idx = if app_state.layout.focused_pane < panes.len() {
            app_state.layout.focused_pane
        } else {
            0
        };

        let mut app = Self {
            config,
            telegram,
            my_user_id,
            chats,
            selected_chat_idx: 0,
            panes,
            focused_pane_idx,
            pane_tree,
            input_history: Vec::new(),
            history_idx: None,
            history_temp: String::new(),
            aliases: app_state.aliases,
            focus_on_chat_list: true,
            status_message: None,
            status_expire: None,
            chat_list_area: None,
            pane_areas: std::collections::HashMap::new(),
            needs_redraw: true,
            show_reactions: app_state.settings.show_reactions,
            show_notifications: app_state.settings.show_notifications,
            compact_mode: app_state.settings.compact_mode,
            show_emojis: app_state.settings.show_emojis,
            show_line_numbers: app_state.settings.show_line_numbers,
            show_timestamps: app_state.settings.show_timestamps,
            show_chat_list: app_state.settings.show_chat_list,
            show_user_colors: app_state.settings.show_user_colors,
            show_borders: app_state.settings.show_borders,
            user_colors: std::collections::HashMap::new(),
        };

        // Load messages for all panes that have a saved chat_id
        // This is what we had before - it works better
        app.load_saved_chat_messages().await?;

        Ok(app)
    }

    /// Refresh messages for a specific pane
    async fn refresh_pane_messages(&mut self, pane_idx: usize) -> Result<()> {
        if let Some(pane) = self.panes.get(pane_idx) {
            if let Some(chat_id) = pane.chat_id {
                match self.telegram.get_messages(chat_id, 50).await {
                    Ok(raw_messages) => {
                        if !raw_messages.is_empty() {
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
                            
                            if let Some(pane) = self.panes.get_mut(pane_idx) {
                                pane.msg_data = msg_data;
                                pane.format_cache.clear(); // Clear cache so messages are re-rendered
                            }
                        }
                    }
                    Err(_) => {
                        // Silently fail - messages will update via polling
                    }
                }
            }
        }
        Ok(())
    }

    /// Load messages for all panes that have a saved chat_id
    async fn load_saved_chat_messages(&mut self) -> Result<()> {
        for (_idx, pane) in self.panes.iter_mut().enumerate() {
            if let Some(chat_id) = pane.chat_id {
                // Try to load messages for this chat
                match self.telegram.get_messages(chat_id, 50).await {
                    Ok(raw_messages) => {
                        if !raw_messages.is_empty() {
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
                            
                            pane.msg_data = msg_data;
                            pane.format_cache.clear(); // Clear cache so messages are re-rendered
                            
                            // Also try to find username from chats list
                            if let Some(chat_info) = self.chats.iter().find(|c| c.id == chat_id) {
                                pane.username = chat_info.username.clone();
                            }
                        }
                    }
                    Err(_) => {
                        // Silently continue loading other panes
                    }
                }
            }
        }
        Ok(())
    }

    pub fn draw(&mut self, f: &mut Frame) {
        // Update cursor blink timer for blinking cursor
        // This will be checked in draw_chat_pane_impl
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

        let colors = [
            Color::Cyan, Color::Yellow, Color::Magenta, Color::Blue,
            Color::Red, Color::Green, Color::White, Color::LightCyan,
            Color::LightYellow, Color::LightMagenta, Color::LightBlue,
            Color::LightRed, Color::LightGreen, Color::DarkGray,
            Color::Rgb(192, 192, 192),
            Color::Rgb(255, 165, 0),
            Color::Rgb(255, 192, 203),
            Color::Rgb(128, 0, 128),
            Color::Rgb(0, 255, 255),
            Color::Rgb(255, 20, 147)
        ];
        
        let mut senders_to_color: Vec<i64> = Vec::new();
        for pane in &self.panes {
            if let Some(chat_id) = pane.chat_id {
                let is_group_chat = self.chats.iter().any(|c| c.id == chat_id && c.is_group);
                if is_group_chat && !pane.msg_data.is_empty() {
                    for msg in &pane.msg_data {
                        if !self.user_colors.contains_key(&msg.sender_id) && !senders_to_color.contains(&msg.sender_id) {
                            senders_to_color.push(msg.sender_id);
                        }
                    }
                }
            }
        }
        
        for &sender_id in &senders_to_color {
            let mut hash = sender_id.abs() as u64;
            hash = hash.wrapping_mul(2654435761);
            hash = hash ^ (hash >> 16);
            hash = hash.wrapping_mul(0x85ebca6b);
            hash = hash ^ (hash >> 13);
            hash = hash.wrapping_mul(0xc2b2ae35);
            hash = hash ^ (hash >> 16);
            
            let color_idx = (hash as usize) % colors.len();
            let color = colors[color_idx];
            self.user_colors.insert(sender_id, color);
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
        // Find which chat is open in the focused pane
        let active_chat_id = self.panes
            .get(self.focused_pane_idx)
            .and_then(|p| p.chat_id);
        
        let items: Vec<ListItem> = self
            .chats
            .iter()
            .enumerate()
            .map(|(_idx, chat)| {
                // Highlight if this chat is open in the focused pane
                let mut style = if Some(chat.id) == active_chat_id {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                // Invert colors for chats with unread messages
                if chat.unread > 0 {
                    style = style.add_modifier(Modifier::REVERSED);
                }

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

        let list_block = if self.show_borders {
            Block::default()
                .borders(Borders::ALL)
                .title("Chats")
                .border_style(border_style)
        } else {
            Block::default()
        };
        let list = List::new(items)
            .block(list_block)
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

        // Calculate input height dynamically based on text width
        let border_overhead = if self.show_borders { 2 } else { 0 };
        let header_height = if self.show_borders { 3 } else { 1 };
        let inner_width = area.width.saturating_sub(if self.show_borders { 2 } else { 0 }).max(1) as usize;
        let text_lines = if is_focused && inner_width > 0 {
            let buf = &pane.input_buffer;
            let mut lines: u16 = 0;
            for line in buf.split('\n') {
                // Each logical line wraps based on its length (+ cursor on last segment)
                let len = line.len();
                lines += ((len as f64) / (inner_width as f64)).ceil().max(1.0) as u16;
            }
            // Account for cursor on the last line
            let last_line_len = buf.rsplit('\n').next().map_or(buf.len(), |l| l.len()) + 1;
            if last_line_len > inner_width {
                let without_cursor = buf.rsplit('\n').next().map_or(buf.len(), |l| l.len());
                let lines_without = ((without_cursor as f64) / (inner_width as f64)).ceil().max(1.0) as u16;
                let lines_with = ((last_line_len as f64) / (inner_width as f64)).ceil().max(1.0) as u16;
                lines += lines_with - lines_without;
            }
            lines.max(1)
        } else {
            1
        };
        let input_height = text_lines + border_overhead + 1; // +1 for spacing below

        let constraints = if has_reply_preview {
            vec![
                Constraint::Length(header_height),
                Constraint::Min(0),     // messages
                Constraint::Length(1),  // reply preview
                Constraint::Length(input_height),
            ]
        } else {
            vec![
                Constraint::Length(header_height),
                Constraint::Min(0),     // messages
                Constraint::Length(input_height),
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
        
        let header_block = if self.show_borders {
            Block::default().borders(Borders::ALL)
        } else {
            Block::default()
        };
        let header = Paragraph::new(header_text)
            .block(header_block)
            .style(header_style);
        f.render_widget(header, chunks[0]);

        // Messages - use rich formatted data if available, otherwise plain messages
        let message_width = chunks[1].width.saturating_sub(4) as usize;
        
        // Check if this is a group chat
        let is_group_chat = if let Some(chat_id) = pane.chat_id {
            self.chats.iter().any(|c| c.id == chat_id && c.is_group)
        } else {
            false
        };
        
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
                
                let (display_msg, style) = if msg.starts_with("[REPLY_TO_ME]") {
                    let clean_msg = msg.replace("[REPLY_TO_ME]", "").trim_start().to_string();
                    (clean_msg, Style::default().fg(Color::Red).add_modifier(Modifier::ITALIC))
                } else if msg.starts_with("  ↳ Reply to") {
                    (msg.to_string(), Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC))
                } else if msg.contains("[OUT]:") {
                    if let Some(marker_pos) = msg.find("[OUT]:") {
                        let prefix = &msg[..marker_pos];
                        let after_marker = &msg[marker_pos + 6..];
                        
                        if let Some(first_colon) = after_marker.find(':') {
                            let sender_id_str = &after_marker[..first_colon];
                            let after_id = &after_marker[first_colon + 1..];
                            if let Some(second_colon) = after_id.find(':') {
                                let sender_name = &after_id[..second_colon];
                                let message_text = &after_id[second_colon + 1..];
                                
                                if let Ok(sender_id) = sender_id_str.parse::<i64>() {
                                    let color = if is_group_chat {
                                        self.user_colors.get(&sender_id).copied().unwrap_or(Color::Green)
                                    } else {
                                        Color::Green
                                    };
                                    let clean_msg = format!("{}{}: {}", prefix, sender_name, message_text);
                                    (clean_msg, Style::default().fg(color))
                                } else {
                                    // Fallback if parsing fails
                                    let parts: Vec<&str> = after_marker.splitn(3, ':').collect();
                                    let clean_msg = if parts.len() >= 3 {
                                        format!("{}{}:{}", prefix, parts[1], parts[2])
                                    } else {
                                        format!("{}{}", prefix, after_marker)
                                    };
                                    (clean_msg, Style::default().fg(Color::Green))
                                }
                            } else {
                                let parts: Vec<&str> = after_marker.splitn(3, ':').collect();
                                let clean_msg = if parts.len() >= 3 {
                                    format!("{}{}:{}", prefix, parts[1], parts[2])
                                } else if parts.len() == 2 {
                                    format!("{}{}:{}", prefix, parts[0], parts[1])
                                } else {
                                    format!("{}{}", prefix, after_marker)
                                };
                                (clean_msg, Style::default().fg(Color::Green))
                            }
                        } else {
                            (format!("{}{}", prefix, after_marker), Style::default().fg(Color::Green))
                        }
                    } else {
                        (msg.to_string(), Style::default().fg(Color::Green))
                    }
                } else if msg.contains("[IN]:") {
                    if let Some(marker_pos) = msg.find("[IN]:") {
                        let prefix = &msg[..marker_pos];
                        let after_marker = &msg[marker_pos + 5..];
                        
                        if let Some(first_colon) = after_marker.find(':') {
                            let sender_id_str = &after_marker[..first_colon];
                            let after_id = &after_marker[first_colon + 1..];
                            if let Some(second_colon) = after_id.find(':') {
                                let sender_name = &after_id[..second_colon];
                                let message_text = &after_id[second_colon + 1..];
                                
                                if let Ok(sender_id) = sender_id_str.parse::<i64>() {
                                    let color = if is_group_chat {
                                        self.user_colors.get(&sender_id).copied().unwrap_or(Color::Cyan)
                                    } else {
                                        Color::Cyan
                                    };
                                    let clean_msg = format!("{}{}: {}", prefix, sender_name, message_text);
                                    (clean_msg, Style::default().fg(color))
                                } else {
                                    // Fallback if parsing fails
                                    let parts: Vec<&str> = after_marker.splitn(3, ':').collect();
                                    let clean_msg = if parts.len() >= 3 {
                                        format!("{}{}:{}", prefix, parts[1], parts[2])
                                    } else {
                                        format!("{}{}", prefix, after_marker)
                                    };
                                    (clean_msg, Style::default().fg(Color::Cyan))
                                }
                            } else {
                                // Fallback if format is wrong
                                let parts: Vec<&str> = after_marker.splitn(3, ':').collect();
                                let clean_msg = if parts.len() >= 3 {
                                    format!("{}{}:{}", prefix, parts[1], parts[2])
                                } else if parts.len() == 2 {
                                    format!("{}{}:{}", prefix, parts[0], parts[1])
                                } else {
                                    format!("{}{}", prefix, after_marker)
                                };
                                (clean_msg, Style::default().fg(Color::Cyan))
                            }
                        } else {
                            // Fallback if format is wrong
                            (format!("{}{}", prefix, after_marker), Style::default().fg(Color::Cyan))
                        }
                    } else {
                        (msg.to_string(), Style::default().fg(Color::Cyan))
                    }
                } else {
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

        let border_lines = if self.show_borders { 2 } else { 1 }; // 1 for spacing above input in borderless
        let available_height = chunks[1].height.saturating_sub(border_lines) as usize;
        let total_lines = message_lines.len();
        
        let actual_scroll = if pane.scroll_offset == 0 && total_lines > available_height {
            total_lines.saturating_sub(available_height)
        } else {
            pane.scroll_offset
        };

        let messages_block = if self.show_borders {
            Block::default().borders(Borders::ALL).title("Messages")
        } else {
            Block::default().padding(Padding::left(2))
        };
        let messages = Paragraph::new(message_lines)
            .block(messages_block)
            .scroll((actual_scroll as u16, 0));
        f.render_widget(messages, chunks[1]);

        if has_reply_preview {
            if let Some(ref preview) = pane.reply_preview {
                let reply_bar = Paragraph::new(preview.as_str())
                    .style(Style::default().fg(Color::Magenta).add_modifier(Modifier::ITALIC));
                f.render_widget(reply_bar, chunks[2]);
            }
        }

        let input_chunk = if has_reply_preview { chunks[3] } else { chunks[2] };
        let input_title = if is_focused && !self.focus_on_chat_list {
            "Input (Alt+Enter for newline, Tab to cycle)"
        } else {
            "Input"
        };
        let mut input_text = if is_focused { pane.input_buffer.clone() } else { String::new() };
        
        // Show block cursor at cursor position when focused
        if is_focused && !self.focus_on_chat_list {
            let cursor_pos = pane.input_cursor.min(input_text.len());
            input_text.insert(cursor_pos, '█');
        }
        
        let input_block = if self.show_borders {
            Block::default().borders(Borders::ALL).title(input_title)
        } else {
            Block::default()
        };
        let input = Paragraph::new(input_text)
            .block(input_block)
            .wrap(Wrap { trim: false });
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

    /// Show a status notification with custom timeout duration
    pub fn notify_with_duration(&mut self, message: &str, duration_secs: u64) {
        self.status_message = Some(message.to_string());
        self.status_expire =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(duration_secs));
    }

    pub async fn open_chat_in_pane(&mut self, pane_idx: usize, chat_id: i64, chat_name: &str) {
        let msg_data = match self.telegram.get_messages(chat_id, 50).await {
            Ok(raw_messages) => raw_messages
                .iter()
                .map(|(msg_id, sender_id, sender_name, text, reply_to_id, media_type, reactions)| {
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
                        reply_to_msg_id: *reply_to_id,
                        reply_sender: None,
                        reply_text: None,
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        };

        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.chat_id = Some(chat_id);
            pane.chat_name = chat_name.to_string();
            pane.msg_data = msg_data;
            pane.messages.clear();
            pane.reply_to_message = None;
            pane.hide_reply_preview();
            pane.scroll_offset = 0;
            pane.format_cache.clear();

            // Set username from chats list if available
            if let Some(chat_info) = self.chats.iter().find(|c| c.id == chat_id) {
                pane.username = chat_info.username.clone();
            }
        }

        // Mark chat as read
        if let Some(chat_info) = self.chats.iter_mut().find(|c| c.id == chat_id) {
            chat_info.unread = 0;
        }
    }

    pub async fn load_pane_messages_if_needed(&mut self, pane_idx: usize) {
        if let Some(pane) = self.panes.get(pane_idx) {
            if let Some(_chat_id) = pane.chat_id {
                if pane.msg_data.is_empty() {
                    let _ = self.refresh_pane_messages(pane_idx).await;
                }
            }
        }
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

    pub fn toggle_split_direction(&mut self) {
        // Find the parent split node that directly contains the focused pane
        if Self::toggle_split_direction_recursive(&mut self.pane_tree, self.focused_pane_idx) {
            self.notify("Split direction toggled");
        } else {
            self.notify("No split to toggle - pane is not in a split");
        }
    }

    fn toggle_split_direction_recursive(node: &mut PaneNode, target_idx: usize) -> bool {
        match node {
            PaneNode::Single(_) => false,
            PaneNode::Split { direction, children } => {
                // Check if target_idx is directly a child of this split (not nested deeper)
                let is_direct_child = children.iter().any(|child| {
                    matches!(child.as_ref(), PaneNode::Single(idx) if *idx == target_idx)
                });

                if is_direct_child {
                    // This is the parent split - toggle its direction
                    *direction = match *direction {
                        SplitDirection::Vertical => SplitDirection::Horizontal,
                        SplitDirection::Horizontal => SplitDirection::Vertical,
                    };
                    true
                } else {
                    // Target might be nested deeper, search in children
                    for child in children.iter_mut() {
                        if Self::toggle_split_direction_recursive(child, target_idx) {
                            return true;
                        }
                    }
                    false
                }
            }
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

    pub fn cycle_focus_reverse(&mut self) {
        let all_panes = self.pane_tree.get_pane_indices();
        if all_panes.is_empty() {
            return;
        }

        if self.focus_on_chat_list {
            // Go to last pane
            self.focus_on_chat_list = false;
            self.focused_pane_idx = *all_panes.last().unwrap();
            self.notify(&format!("Focus: Pane #{}", self.focused_pane_idx + 1));
        } else {
            if let Some(current_pos) = all_panes.iter().position(|&idx| idx == self.focused_pane_idx) {
                if current_pos > 0 {
                    self.focused_pane_idx = all_panes[current_pos - 1];
                    self.notify(&format!("Focus: Pane #{}", self.focused_pane_idx + 1));
                } else {
                    self.focus_on_chat_list = true;
                    self.notify("Focus: Chat List");
                }
            }
        }
    }

    pub fn focus_next_pane(&mut self) {
        let all_panes = self.pane_tree.get_pane_indices();
        if all_panes.len() < 2 {
            return;
        }
        if let Some(current_pos) = all_panes.iter().position(|&idx| idx == self.focused_pane_idx) {
            let next = (current_pos + 1) % all_panes.len();
            self.focused_pane_idx = all_panes[next];
            self.focus_on_chat_list = false;
            self.notify(&format!("Focus: Pane #{}", self.focused_pane_idx + 1));
        }
    }

    pub fn focus_prev_pane(&mut self) {
        let all_panes = self.pane_tree.get_pane_indices();
        if all_panes.len() < 2 {
            return;
        }
        if let Some(current_pos) = all_panes.iter().position(|&idx| idx == self.focused_pane_idx) {
            let prev = if current_pos > 0 { current_pos - 1 } else { all_panes.len() - 1 };
            self.focused_pane_idx = all_panes[prev];
            self.focus_on_chat_list = false;
            self.notify(&format!("Focus: Pane #{}", self.focused_pane_idx + 1));
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

    pub fn toggle_user_colors(&mut self) {
        self.show_user_colors = !self.show_user_colors;
        let status = if self.show_user_colors { "ON" } else { "OFF" };
        self.notify(&format!("User colors: {}", status));
        self.refresh_all_pane_displays();
    }

    pub fn toggle_borders(&mut self) {
        self.show_borders = !self.show_borders;
        self.notify(&format!("Borders: {}", if self.show_borders { "ON" } else { "OFF" }));
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
        // Each chat item is 1 line, starting at list_area.y + border_offset (after top border if present)
        let border_offset = if self.show_borders { 1 } else { 0 };
        if y < list_area.y + border_offset || y >= list_area.y + list_area.height - border_offset {
            return Ok(()); // Clicked on border or outside
        }
        
        let relative_y = (y - list_area.y - border_offset) as usize;
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
                    pane.messages.clear(); // Clear status messages when switching chats
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
                            pane.input_cursor = pane.input_buffer.len();
                    }
                        Some(idx) if idx > 0 => {
                            self.history_idx = Some(idx - 1);
                            pane.input_buffer = self.input_history[idx - 1].clone();
                            pane.input_cursor = pane.input_buffer.len();
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
                        pane.input_cursor = pane.input_buffer.len();
                    } else {
                        // Back to current input
                        self.history_idx = None;
                        pane.input_buffer = self.history_temp.clone();
                        pane.input_cursor = pane.input_buffer.len();
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
                pane.input_cursor = pane.input_buffer.len();
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
                        pane.messages.clear(); // Clear status messages when switching chats
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
            let (input_text, _chat_id, _reply_to_id) = if let Some(pane) = self.panes.get(self.focused_pane_idx) {
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
                    pane.input_cursor = 0;
                    }
                    return Ok(());
                }
            }

            // Handle reply mode or normal send
            if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
                if let (Some(chat_id), Some(reply_to_id)) =
                    (pane.chat_id, pane.reply_to_message)
                {
                    // FIRST: Add message DIRECTLY to pane IMMEDIATELY - no waiting!
                    let new_msg = crate::widgets::MessageData {
                        msg_id: 0, // Temporary ID
                        sender_id: self.my_user_id,
                        sender_name: "You".to_string(),
                        text: input_text.clone(),
                        is_outgoing: true,
                        timestamp: chrono::Utc::now().timestamp(),
                        media_type: None,
                        media_label: None,
                        reactions: std::collections::HashMap::new(),
                        reply_to_msg_id: Some(reply_to_id),
                        reply_sender: None,
                        reply_text: None,
                    };
                    pane.msg_data.push(new_msg);
                    pane.format_cache.clear();
                    
                    pane.reply_to_message = None;
                    pane.hide_reply_preview();
                    pane.input_buffer.clear();
                    pane.input_cursor = 0;
                    
                    // THEN: Send message in background - don't wait!
                    let telegram = self.telegram.clone();
                    let chat_id_copy = chat_id;
                    let reply_to_id_copy = reply_to_id;
                    let input_text_copy = input_text.clone();
                    tokio::spawn(async move {
                        let _ = telegram.reply_to_message(chat_id_copy, reply_to_id_copy, &input_text_copy).await;
                    });
                } else if let Some(chat_id) = pane.chat_id {
                    // FIRST: Add message DIRECTLY to pane IMMEDIATELY - no waiting!
                    let new_msg = crate::widgets::MessageData {
                        msg_id: 0, // Temporary ID
                        sender_id: self.my_user_id,
                        sender_name: "You".to_string(),
                        text: input_text.clone(),
                        is_outgoing: true,
                        timestamp: chrono::Utc::now().timestamp(),
                        media_type: None,
                        media_label: None,
                        reactions: std::collections::HashMap::new(),
                        reply_to_msg_id: None,
                        reply_sender: None,
                        reply_text: None,
                    };
                    pane.msg_data.push(new_msg);
                    pane.format_cache.clear();
                    
                    pane.input_buffer.clear();
                    pane.input_cursor = 0;
                    
                    // THEN: Send message in background - don't wait!
                    let telegram = self.telegram.clone();
                    let chat_id_copy = chat_id;
                    let input_text_copy = input_text.clone();
                    tokio::spawn(async move {
                        let _ = telegram.send_message(chat_id_copy, &input_text_copy).await;
                    });
                }
            }
        }
        Ok(())
    }

    pub fn handle_char(&mut self, c: char) {
        if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
            pane.input_buffer.insert(pane.input_cursor, c);
            pane.input_cursor += c.len_utf8();
        }
        self.history_idx = None;
    }

    pub fn handle_backspace(&mut self) {
        if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
            if pane.input_cursor > 0 {
                let prev = pane.input_buffer[..pane.input_cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                pane.input_buffer.remove(prev);
                pane.input_cursor = prev;
            }
        }
        self.history_idx = None;
    }

    pub fn handle_delete(&mut self) {
        if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
            if pane.input_cursor < pane.input_buffer.len() {
                pane.input_buffer.remove(pane.input_cursor);
            }
        }
    }

    pub fn handle_input_left(&mut self) {
        if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
            if pane.input_cursor > 0 {
                pane.input_cursor = pane.input_buffer[..pane.input_cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
            }
        }
    }

    pub fn handle_input_right(&mut self) {
        if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
            if pane.input_cursor < pane.input_buffer.len() {
                pane.input_cursor = pane.input_buffer[pane.input_cursor..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| pane.input_cursor + i)
                    .unwrap_or(pane.input_buffer.len());
            }
        }
    }

    pub fn handle_home(&mut self) {
        if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
            pane.input_cursor = 0;
        }
    }

    pub fn handle_end(&mut self) {
        if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
            pane.input_cursor = pane.input_buffer.len();
        }
    }

    // =========================================================================
    // New message handling
    // =========================================================================

    pub async fn process_telegram_events(&mut self) -> Result<bool> {
        // Process incoming updates
        let updates = self.telegram.poll_updates().await?;
        let had_updates = !updates.is_empty();

        for update in updates {
            match update {
                crate::telegram::TelegramUpdate::NewMessage {
                    chat_id,
                    _sender_name: _,
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
                        // For outgoing messages, don't reload immediately - they're already shown optimistically
                        // Only reload for incoming messages
                        if !is_outgoing {
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
                                        pane.format_cache.clear(); // Clear cache so messages are re-rendered
                                        // Don't clear messages - they may contain status messages
                                    }
                                }
                            }
                        }
                        // For outgoing messages, the optimistic message is already shown
                        // It will be updated naturally when we refresh later
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

        Ok(had_updates)
    }

    // =========================================================================
    // State persistence
    // =========================================================================

    pub fn save_state(&self) -> Result<()> {
        let layout = LayoutData {
            panes: self
                .panes
                .iter()
                .map(|p| {
                    let filter_type_str = p.filter_type.as_ref().map(|ft| match ft {
                        crate::widgets::FilterType::Sender => "sender".to_string(),
                        crate::widgets::FilterType::Media => "media".to_string(),
                        crate::widgets::FilterType::Link => "link".to_string(),
                    });
                    PaneState {
                        chat_id: p.chat_id,
                        chat_name: p.chat_name.clone(),
                        scroll_offset: p.scroll_offset,
                        filter_type: filter_type_str,
                        filter_value: p.filter_value.clone(),
                    }
                })
                .collect(),
            focused_pane: self.focused_pane_idx,
            pane_tree: Some(self.pane_tree.clone()),
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
        config.settings.show_user_colors = self.show_user_colors;
        config.settings.show_borders = self.show_borders;
        config.settings.show_chat_list = self.show_chat_list;
        config.save()?;

        Ok(())
    }
}

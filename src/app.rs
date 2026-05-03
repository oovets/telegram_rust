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
use crate::kitty_preview;
use crate::persistence::{Aliases, AppState, LayoutData, PaneState};
use crate::split_view::{PaneNode, SplitDirection};
use crate::telegram::TelegramClient;
use crate::utils::{send_desktop_notification, try_autocomplete};
use crate::widgets::ChatPane;

pub struct App {
    pub config: Config,
    pub telegram: TelegramClient,
    pub my_user_id: i64,
    pub chats: Vec<ChatInfo>,
    pub selected_chat_idx: usize,
    pub panes: Vec<ChatPane>,
    pub focused_pane_idx: usize,
    pub pane_tree: PaneNode,
    pub input_history: Vec<String>,
    pub history_idx: Option<usize>,
    pub history_temp: String,
    pub aliases: Aliases,
    pub focus_on_chat_list: bool,
    pub status_message: Option<String>,
    pub status_expire: Option<std::time::Instant>,
    pub pane_areas: std::collections::HashMap<usize, Rect>,
    pub chat_list_area: Option<Rect>,
    pub needs_redraw: bool,

    pub show_reactions: bool,
    pub show_notifications: bool,
    pub compact_mode: bool,
    pub show_emojis: bool,
    pub show_line_numbers: bool,
    pub show_timestamps: bool,
    pub show_chat_list: bool,
    pub chat_list_width: u16,
    pub show_unread_count: bool,
    pub show_user_colors: bool,
    pub show_borders: bool,
    pub user_colors: std::collections::HashMap<i64, Color>,
    pub muted_chat_ids: std::collections::HashSet<i64>,
    pub inline_preview_path: Option<String>,
    pub inline_preview_name: Option<String>,
    pub inline_preview_rect: Option<(u16, u16, u16, u16)>,
    pub inline_preview_dirty: bool,
    pub inline_preview_last_sig: Option<String>,
    pub inline_preview_zoom_pct: u16,
    pub inline_preview_chat_id: Option<i64>,
    pub inline_preview_image_msg_ids: Vec<i32>,
    pub inline_preview_index: Option<usize>,
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

        let chats = telegram.get_dialogs().await.unwrap_or_else(|_| Vec::new());

        let (pane_tree, required_indices) = if let Some(saved_tree) = app_state.layout.pane_tree {
            let indices = saved_tree.get_pane_indices();
            (saved_tree, indices)
        } else {
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
        
        let max_required_idx = required_indices.iter().max().copied().unwrap_or(0);
        let total_panes_needed = (max_required_idx + 1).max(app_state.layout.panes.len()).max(1);
        
        let mut panes: Vec<ChatPane> = Vec::new();
        for i in 0..total_panes_needed {
            if let Some(ps) = app_state.layout.panes.get(i) {
                let mut pane = ChatPane::new();
                pane.chat_id = ps.chat_id;
                pane.chat_name = ps.chat_name.clone();
                pane.scroll_offset = ps.scroll_offset;
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
            chat_list_width: app_state.settings.chat_list_width.max(10),
            show_unread_count: app_state.settings.show_unread_count,
            show_user_colors: app_state.settings.show_user_colors,
            show_borders: app_state.settings.show_borders,
            user_colors: std::collections::HashMap::new(),
            muted_chat_ids: app_state.layout.muted_chat_ids.iter().copied().collect(),
            inline_preview_path: None,
            inline_preview_name: None,
            inline_preview_rect: None,
            inline_preview_dirty: false,
            inline_preview_last_sig: None,
            inline_preview_zoom_pct: 100,
            inline_preview_chat_id: None,
            inline_preview_image_msg_ids: Vec::new(),
            inline_preview_index: None,
        };

        app.load_saved_chat_messages().await?;

        Ok(app)
    }

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
                                pane.format_cache.clear();
                            }
                        }
                    }
                    Err(_) => {
                    }
                }
            }
        }
        Ok(())
    }

    async fn load_saved_chat_messages(&mut self) -> Result<()> {
        for (_idx, pane) in self.panes.iter_mut().enumerate() {
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
                            
                            pane.msg_data = msg_data;
                            pane.format_cache.clear();
                            
                            if let Some(chat_info) = self.chats.iter().find(|c| c.id == chat_id) {
                                pane.username = chat_info.username.clone();
                            }
                        }
                    }
                    Err(_) => {
                    }
                }
            }
        }
        Ok(())
    }

    pub fn draw(&mut self, f: &mut Frame) {
        for pane in &mut self.panes {
            pane.check_typing_expired();
        }
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
            let total_width = outer[0].width;
            let min_pane_width = 20;
            let max_chat_width = total_width.saturating_sub(min_pane_width).max(10);
            let chat_width = self.chat_list_width.clamp(10, max_chat_width);
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(chat_width), Constraint::Min(0)])
                .split(outer[0]);
            (Some(chunks[0]), chunks[1])
        } else {
            (None, outer[0])
        };

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

        let mut render_pane_area = pane_area;
        if self.inline_preview_path.is_some() {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(pane_area);
            render_pane_area = split[0];
            let preview_panel = split[1];
            let preview_title = self
                .inline_preview_name
                .as_deref()
                .unwrap_or("Inline image preview");
            let zoom = self.inline_preview_zoom_pct;
            let idx_info = if let (Some(i), total) = (self.inline_preview_index, self.inline_preview_image_msg_ids.len()) {
                if total > 0 {
                    format!(" [{}/{}]", i + 1, total)
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!("Preview: {} ({}%){}", preview_title, zoom, idx_info));
            f.render_widget(block, preview_panel);

            let inner = preview_panel.inner(ratatui::layout::Margin {
                horizontal: if self.show_borders { 1 } else { 0 },
                vertical: if self.show_borders { 1 } else { 0 },
            });
            let col = inner.x.saturating_add(1);
            let row = inner.y.saturating_add(1);
            let cols = inner.width.saturating_sub(1).max(1);
            let rows = inner.height.saturating_sub(1).max(1);
            self.inline_preview_rect = Some((col, row, cols, rows));
        } else {
            self.inline_preview_rect = None;
        }

        let render_fn = |f: &mut Frame, area: Rect, pane: &ChatPane, is_focused: bool| {
            self.draw_chat_pane_impl(f, area, pane, is_focused);
        };

        let mut pane_areas = std::collections::HashMap::new();
        self.pane_tree
            .render(
                f,
                render_pane_area,
                &self.panes,
                self.focused_pane_idx,
                &render_fn,
                &mut pane_areas,
            );
        self.pane_areas = pane_areas;

        if has_status {
            if let Some(ref msg) = self.status_message {
                let status = Paragraph::new(msg.as_str())
                    .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
                f.render_widget(status, outer[1]);
            }
        }
    }

    fn draw_chat_list(&self, f: &mut Frame, area: Rect) {
        let active_chat_id = self.panes
            .get(self.focused_pane_idx)
            .and_then(|p| p.chat_id);
        
        let max_width = area.width.saturating_sub(6).max(1) as usize;
        let (unread_group, active_group, muted_group, other_group) = self.chat_list_groups();

        let build_item = |chat: &ChatInfo| -> ListItem {
            let base_style = if Some(chat.id) == active_chat_id {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let unread_marker = if chat.unread > 0 { "▶ " } else { "" };
            let unread_count = if chat.unread > 0 {
                if self.show_unread_count {
                    format!("({}) ", chat.unread)
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let mut name_part = chat.name.clone();
            if let Some(ref username) = chat.username {
                if !username.is_empty() {
                    name_part.push_str(&format!(" {}", username));
                }
            }

            let mut spans = Vec::new();
            if !unread_marker.is_empty() {
                spans.push(ratatui::text::Span::styled(
                    unread_marker.to_string(),
                    Style::default().fg(Color::Red),
                ));
            }
            if !unread_count.is_empty() {
                spans.push(ratatui::text::Span::styled(unread_count, base_style));
            }
            spans.push(ratatui::text::Span::styled(name_part, base_style));

            let total_chars: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            let truncated = total_chars > max_width && max_width > 0;
            let mut remaining = if truncated { max_width.saturating_sub(1) } else { max_width };
            let mut out_spans: Vec<ratatui::text::Span> = Vec::new();

            for span in spans.into_iter() {
                if remaining == 0 {
                    break;
                }
                let span_len = span.content.chars().count();
                if span_len <= remaining {
                    remaining = remaining.saturating_sub(span_len);
                    out_spans.push(span);
                } else {
                    let clipped: String = span.content.chars().take(remaining).collect();
                    out_spans.push(ratatui::text::Span::styled(clipped, span.style));
                    break;
                }
            }

            if truncated {
                out_spans.push(ratatui::text::Span::styled("…".to_string(), base_style));
            }

            ListItem::new(ratatui::text::Line::from(out_spans))
        };

        let header_style = Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD);
        let mut items: Vec<ListItem> = Vec::new();

        if !unread_group.is_empty() {
            items.push(ListItem::new("Unread").style(header_style));
            for chat_idx in unread_group.iter() {
                items.push(build_item(&self.chats[*chat_idx]));
            }
        }

        if !active_group.is_empty() {
            items.push(ListItem::new("Active").style(header_style));
            for chat_idx in active_group.iter() {
                items.push(build_item(&self.chats[*chat_idx]));
            }
        }

        if !other_group.is_empty() {
            items.push(ListItem::new("Other").style(header_style));
            for chat_idx in other_group.iter() {
                items.push(build_item(&self.chats[*chat_idx]));
            }
        }

        if !muted_group.is_empty() {
            items.push(ListItem::new("Muted").style(header_style));
            for chat_idx in muted_group.iter() {
                items.push(build_item(&self.chats[*chat_idx]));
            }
        }

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

        let border_overhead = if self.show_borders { 2 } else { 0 };
        let header_height = if self.show_borders { 3 } else { 1 };
        let inner_width = area.width.saturating_sub(if self.show_borders { 2 } else { 0 }).max(1) as usize;
        let text_lines = if is_focused && inner_width > 0 {
            let buf = &pane.input_buffer;
            let mut lines: u16 = 0;
            for line in buf.split('\n') {
                let len = line.len();
                lines += ((len as f64) / (inner_width as f64)).ceil().max(1.0) as u16;
            }
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
        let input_height = text_lines + border_overhead + 1;

        let constraints = if has_reply_preview {
            vec![
                Constraint::Length(header_height),
                Constraint::Min(0),
                Constraint::Length(1),
                Constraint::Length(input_height),
            ]
        } else {
            vec![
                Constraint::Length(header_height),
                Constraint::Min(0),
                Constraint::Length(input_height),
            ]
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        let header_style = if is_focused {
            if self.focus_on_chat_list {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
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

        let message_width = chunks[1].width.saturating_sub(4) as usize;
        
        let is_group_chat = if let Some(chat_id) = pane.chat_id {
            self.chats.iter().any(|c| c.id == chat_id && c.is_group)
        } else {
            false
        };
        
        let display_lines = if !pane.msg_data.is_empty() {
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
                self.show_unread_count,
                &self.aliases.map,
            );
            
            if !pane.messages.is_empty() {
                lines.push(String::new());
                lines.extend(pane.messages.clone());
            }
            lines
        } else {
            pane.messages.clone()
        };

        let wrap_plain_text = |text: &str, max_width: usize| -> Vec<String> {
            if max_width == 0 || text.len() <= max_width {
                return vec![text.to_string()];
            }

            let mut lines = Vec::new();
            let mut current_line = String::new();

            for word in text.split_whitespace() {
                if current_line.len() + word.len() + 1 > max_width {
                    if !current_line.is_empty() {
                        lines.push(current_line.clone());
                        current_line.clear();
                    }
                    if word.chars().count() > max_width {
                        let split_at = word
                            .char_indices()
                            .nth(max_width)
                            .map(|(i, _)| i)
                            .unwrap_or(word.len());
                        lines.push(word[..split_at].to_string());
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
                lines.push(current_line);
            }
            lines
        };

        let wrap_message_with_indent =
            |prefix: &str, sender_name: &str, message_text: &str, max_width: usize| -> Vec<String> {
                let header = format!("{}{}: ", prefix, sender_name);
                let indent_len = header.chars().count();

                if max_width == 0 {
                    return vec![format!("{}{}", header, message_text)];
                }

                if indent_len >= max_width {
                    return wrap_plain_text(&format!("{}{}", header, message_text), max_width);
                }

                let first_width = max_width.saturating_sub(indent_len);
                let wrapped = wrap_plain_text(message_text, first_width);
                if wrapped.is_empty() {
                    return vec![header.trim_end().to_string()];
                }

                let indent = " ".repeat(indent_len);
                let mut lines = Vec::with_capacity(wrapped.len());
                lines.push(format!("{}{}", header, wrapped[0]));
                for line in wrapped.iter().skip(1) {
                    lines.push(format!("{}{}", indent, line));
                }
                lines
            };

        let style_name_in_line = |line: &str, sender_name: &str, name_style: Style| -> Line {
            if sender_name.is_empty() {
                return Line::from(line.to_string());
            }

            let name_token = format!("{}:", sender_name);
            if let Some(start) = line.find(&name_token) {
                let name_end = start + sender_name.len();
                let before = &line[..start];
                let name = &line[start..name_end];
                let after = &line[name_end..];
                Line::from(vec![
                    ratatui::text::Span::raw(before.to_string()),
                    ratatui::text::Span::styled(name.to_string(), name_style),
                    ratatui::text::Span::raw(after.to_string()),
                ])
            } else {
                Line::from(line.to_string())
            }
        };

        let message_lines: Vec<Line> = display_lines
            .iter()
            .flat_map(|msg| {
                if msg.is_empty() {
                    return vec![Line::from("")];
                }

                if msg.starts_with("[REPLY_TO_ME]") {
                    let clean_msg = msg.replace("[REPLY_TO_ME]", "").trim_start().to_string();
                    return wrap_plain_text(&clean_msg, message_width)
                        .into_iter()
                        .map(|line| {
                            Line::from(line).style(
                                Style::default()
                                    .fg(Color::Red)
                                    .add_modifier(Modifier::ITALIC),
                            )
                        })
                        .collect();
                }

                if msg.starts_with("  ↳ Reply to") {
                    return wrap_plain_text(msg, message_width)
                        .into_iter()
                        .map(|line| {
                            Line::from(line).style(
                                Style::default()
                                    .fg(Color::DarkGray)
                                    .add_modifier(Modifier::ITALIC),
                            )
                        })
                        .collect();
                }

                if msg.contains("[OUT]:") || msg.contains("[IN]:") {
                    let is_outgoing = msg.contains("[OUT]:");
                    let marker = if is_outgoing { "[OUT]:" } else { "[IN]:" };
                    let marker_len = marker.len();
                    if let Some(marker_pos) = msg.find(marker) {
                        let prefix = &msg[..marker_pos];
                        let after_marker = &msg[marker_pos + marker_len..];

                        if let Some(first_colon) = after_marker.find(':') {
                            let sender_id_str = &after_marker[..first_colon];
                            let after_id = &after_marker[first_colon + 1..];
                            if let Some(second_colon) = after_id.find(':') {
                                let sender_name = &after_id[..second_colon];
                                let message_text = &after_id[second_colon + 1..];

                                if let Ok(sender_id) = sender_id_str.parse::<i64>() {
                                    let base_color = if is_outgoing {
                                        Color::Green
                                    } else {
                                        Color::Cyan
                                    };
                                    let color = if is_group_chat {
                                        self.user_colors.get(&sender_id).copied().unwrap_or(base_color)
                                    } else {
                                        base_color
                                    };
                                    let lines = wrap_message_with_indent(
                                        prefix,
                                        sender_name,
                                        message_text,
                                        message_width,
                                    );
                                    if self.show_user_colors {
                                        return lines
                                            .into_iter()
                                            .enumerate()
                                            .map(|(idx, line)| {
                                                if idx == 0 {
                                                    style_name_in_line(
                                                        &line,
                                                        sender_name,
                                                        Style::default().fg(color),
                                                    )
                                                } else {
                                                    Line::from(line)
                                                }
                                            })
                                            .collect();
                                    }
                                    return lines.into_iter().map(Line::from).collect();
                                }
                            }
                        }
                    }
                }

                wrap_plain_text(msg, message_width)
                    .into_iter()
                    .map(Line::from)
                    .collect()
            })
            .collect();

        let border_lines = if self.show_borders { 2 } else { 1 };
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
        Ok(())
    }

    pub fn notify(&mut self, message: &str) {
        self.status_message = Some(message.to_string());
        self.status_expire =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
    }

    pub fn notify_with_duration(&mut self, message: &str, duration_secs: u64) {
        self.status_message = Some(message.to_string());
        self.status_expire =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(duration_secs));
    }

    pub fn open_inline_preview(&mut self, path: String) {
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "image".to_string());
        self.inline_preview_path = Some(path);
        self.inline_preview_name = Some(name);
        self.inline_preview_dirty = true;
        self.inline_preview_last_sig = None;
        self.needs_redraw = true;
    }

    pub fn open_inline_preview_for_message(
        &mut self,
        pane_idx: usize,
        chat_id: i64,
        message_id: i32,
        path: String,
    ) {
        self.open_inline_preview(path);
        self.inline_preview_chat_id = Some(chat_id);
        self.inline_preview_zoom_pct = 100;

        let mut image_ids = Vec::new();
        if let Some(pane) = self.panes.get(pane_idx) {
            for m in &pane.msg_data {
                let is_image = matches!(m.media_type.as_deref(), Some("photo" | "sticker" | "gif"));
                if is_image {
                    image_ids.push(m.msg_id);
                }
            }
        }
        self.inline_preview_index = image_ids.iter().position(|&id| id == message_id);
        self.inline_preview_image_msg_ids = image_ids;
    }

    pub fn close_inline_preview(&mut self) {
        self.inline_preview_path = None;
        self.inline_preview_name = None;
        self.inline_preview_rect = None;
        self.inline_preview_dirty = false;
        self.inline_preview_last_sig = None;
        self.inline_preview_zoom_pct = 100;
        self.inline_preview_chat_id = None;
        self.inline_preview_image_msg_ids.clear();
        self.inline_preview_index = None;
        let _ = kitty_preview::clear_all_images();
        self.needs_redraw = true;
    }

    pub fn zoom_inline_preview_in(&mut self) {
        self.inline_preview_zoom_pct = (self.inline_preview_zoom_pct.saturating_add(10)).min(300);
        self.inline_preview_dirty = true;
        self.needs_redraw = true;
    }

    pub fn zoom_inline_preview_out(&mut self) {
        self.inline_preview_zoom_pct = self.inline_preview_zoom_pct.saturating_sub(10).max(30);
        self.inline_preview_dirty = true;
        self.needs_redraw = true;
    }

    fn image_path_for_preview(&mut self, source_path: &str, chat_id: i64, msg_id: i32) -> String {
        if source_path.to_ascii_lowercase().ends_with(".png") {
            return source_path.to_string();
        }
        match image::open(source_path) {
            Ok(img) => {
                let png_path = std::env::temp_dir().join(format!(
                    "telegram_preview_{}_{}.png",
                    chat_id, msg_id
                ));
                if let Err(e) = img.save_with_format(&png_path, image::ImageFormat::Png) {
                    self.notify(&format!("Preview conversion failed: {}", e));
                    source_path.to_string()
                } else {
                    png_path.to_string_lossy().to_string()
                }
            }
            Err(e) => {
                self.notify(&format!("Preview decode failed: {}", e));
                source_path.to_string()
            }
        }
    }

    pub async fn preview_next_image(&mut self) -> Result<()> {
        self.preview_step_image(1).await
    }

    pub async fn preview_prev_image(&mut self) -> Result<()> {
        self.preview_step_image(-1).await
    }

    async fn preview_step_image(&mut self, delta: i32) -> Result<()> {
        let Some(chat_id) = self.inline_preview_chat_id else {
            return Ok(());
        };
        if self.inline_preview_image_msg_ids.is_empty() {
            self.notify("No image list available for preview");
            return Ok(());
        }

        let current = self.inline_preview_index.unwrap_or(0);
        let total = self.inline_preview_image_msg_ids.len() as i32;
        let mut next = current as i32 + delta;
        if next < 0 {
            next = total - 1;
        } else if next >= total {
            next = 0;
        }
        let next_usize = next as usize;
        let msg_id = self.inline_preview_image_msg_ids[next_usize];
        self.notify("Loading next image...");
        let downloaded = self
            .telegram
            .download_media_by_id(chat_id, msg_id, &std::env::temp_dir())
            .await?;
        let preview_path = self.image_path_for_preview(&downloaded, chat_id, msg_id);
        self.inline_preview_index = Some(next_usize);
        self.inline_preview_name = std::path::Path::new(&preview_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string());
        self.inline_preview_path = Some(preview_path);
        self.inline_preview_dirty = true;
        self.needs_redraw = true;
        Ok(())
    }

    pub fn maybe_render_inline_preview(&mut self) {
        let Some(path) = self.inline_preview_path.clone() else {
            return;
        };
        let Some((col, row, cols, rows)) = self.inline_preview_rect else {
            return;
        };
        let scaled_cols = (((cols as u32) * (self.inline_preview_zoom_pct as u32)) / 100)
            .max(1)
            .min((cols as u32) * 3) as u16;
        let scaled_rows = (((rows as u32) * (self.inline_preview_zoom_pct as u32)) / 100)
            .max(1)
            .min((rows as u32) * 3) as u16;

        let sig = format!(
            "{}:{}:{}:{}:{}:{}",
            path, col, row, scaled_cols, scaled_rows, self.inline_preview_zoom_pct
        );
        if !self.inline_preview_dirty && self.inline_preview_last_sig.as_deref() == Some(&sig) {
            return;
        }
        if let Err(e) = kitty_preview::clear_all_images() {
            self.notify(&format!("Preview clear failed: {}", e));
            return;
        }
        if let Err(e) = kitty_preview::render_png_at(&path, col, row, scaled_cols, scaled_rows) {
            self.notify(&format!("Inline preview failed: {}", e));
            return;
        }
        self.inline_preview_dirty = false;
        self.inline_preview_last_sig = Some(sig);
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

            if let Some(chat_info) = self.chats.iter().find(|c| c.id == chat_id) {
                pane.username = chat_info.username.clone();
            }
        }

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
        if Self::toggle_split_direction_recursive(&mut self.pane_tree, self.focused_pane_idx) {
        } else {
            self.notify("No split to toggle - pane is not in a split");
        }
    }

    fn toggle_split_direction_recursive(node: &mut PaneNode, target_idx: usize) -> bool {
        match node {
            PaneNode::Single(_) => false,
            PaneNode::Split { direction, children } => {
                let is_direct_child = children.iter().any(|child| {
                    matches!(child.as_ref(), PaneNode::Single(idx) if *idx == target_idx)
                });

                if is_direct_child {
                    *direction = match *direction {
                        SplitDirection::Vertical => SplitDirection::Horizontal,
                        SplitDirection::Horizontal => SplitDirection::Vertical,
                    };
                    true
                } else {
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
            self.focus_on_chat_list = false;
            self.focused_pane_idx = all_panes[0];
            self.mark_pane_chat_read(self.focused_pane_idx);
        } else {
            if let Some(current_pos) = all_panes.iter().position(|&idx| idx == self.focused_pane_idx) {
                if current_pos + 1 < all_panes.len() {
                    self.focused_pane_idx = all_panes[current_pos + 1];
                    self.mark_pane_chat_read(self.focused_pane_idx);
                } else {
                    self.focus_on_chat_list = true;
                }
            } else {
                self.focused_pane_idx = all_panes[0];
                self.mark_pane_chat_read(self.focused_pane_idx);
            }
        }
    }

    pub fn cycle_focus_reverse(&mut self) {
        let all_panes = self.pane_tree.get_pane_indices();
        if all_panes.is_empty() {
            return;
        }

        if self.focus_on_chat_list {
            self.focus_on_chat_list = false;
            self.focused_pane_idx = *all_panes.last().unwrap();
            self.mark_pane_chat_read(self.focused_pane_idx);
        } else {
            if let Some(current_pos) = all_panes.iter().position(|&idx| idx == self.focused_pane_idx) {
                if current_pos > 0 {
                    self.focused_pane_idx = all_panes[current_pos - 1];
                    self.mark_pane_chat_read(self.focused_pane_idx);
                } else {
                    self.focus_on_chat_list = true;
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
            self.mark_pane_chat_read(self.focused_pane_idx);
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
            self.mark_pane_chat_read(self.focused_pane_idx);
        }
    }


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

    pub fn toggle_unread_count(&mut self) {
        self.show_unread_count = !self.show_unread_count;
        self.notify(&format!(
            "Unread count: {}",
            if self.show_unread_count { "ON" } else { "OFF" }
        ));
        self.refresh_all_pane_displays();
    }

    pub fn resize_chat_list_narrower(&mut self) {
        self.chat_list_width = self.chat_list_width.saturating_sub(2).max(10);
        self.notify(&format!("Sidebar width: {}", self.chat_list_width));
    }

    pub fn resize_chat_list_wider(&mut self) {
        self.chat_list_width = self.chat_list_width.saturating_add(2).max(10);
        self.notify(&format!("Sidebar width: {}", self.chat_list_width));
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

    pub fn toggle_mute_selected_chat(&mut self) {
        let chat_id = if self.focus_on_chat_list {
            let ordered = self.chat_list_order();
            if let Some(&chat_idx) = ordered.get(self.selected_chat_idx) {
                self.chats.get(chat_idx).map(|c| c.id)
            } else {
                None
            }
        } else {
            self.panes.get(self.focused_pane_idx).and_then(|p| p.chat_id)
        };

        let Some(chat_id) = chat_id else {
            self.notify("No chat selected to mute");
            return;
        };

        let chat_name = self
            .chats
            .iter()
            .find(|c| c.id == chat_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "Chat".to_string());

        if self.muted_chat_ids.remove(&chat_id) {
            self.notify(&format!("Unmuted: {}", chat_name));
        } else {
            self.muted_chat_ids.insert(chat_id);
            self.notify(&format!("Muted: {}", chat_name));
        }

        if let Err(e) = self.save_state() {
            self.notify(&format!("Failed to persist mute: {}", e));
        }
    }

    fn chat_list_groups(&self) -> (Vec<usize>, Vec<usize>, Vec<usize>, Vec<usize>) {
        let mut open_chat_ids = std::collections::HashSet::new();
        for pane in &self.panes {
            if let Some(chat_id) = pane.chat_id {
                open_chat_ids.insert(chat_id);
            }
        }

        let mut unread = Vec::new();
        let mut active = Vec::new();
        let mut muted = Vec::new();
        let mut other = Vec::new();

        for (idx, chat) in self.chats.iter().enumerate() {
            if open_chat_ids.contains(&chat.id) {
                active.push(idx);
            } else if self.muted_chat_ids.contains(&chat.id) {
                muted.push(idx);
            } else if chat.unread > 0 {
                unread.push(idx);
            } else {
                other.push(idx);
            }
        }

        (unread, active, muted, other)
    }

    fn chat_list_order(&self) -> Vec<usize> {
        let (unread, active, muted, other) = self.chat_list_groups();
        let mut ordered = Vec::with_capacity(self.chats.len());
        ordered.extend(unread);
        ordered.extend(active);
        ordered.extend(other);
        ordered.extend(muted);
        ordered
    }

    fn mark_pane_chat_read(&mut self, pane_idx: usize) {
        let chat_id = match self.panes.get(pane_idx).and_then(|p| p.chat_id) {
            Some(chat_id) => chat_id,
            None => return,
        };

        if let Some(chat_info) = self.chats.iter_mut().find(|c| c.id == chat_id) {
            chat_info.unread = 0;
        }

        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.unread_count_at_load = 0;
        }
    }


    pub fn handle_mouse_click(&mut self, x: u16, y: u16) {
        for (&pane_idx, &area) in &self.pane_areas {
            if x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height {
                self.focused_pane_idx = pane_idx;
                self.focus_on_chat_list = false;
                self.mark_pane_chat_read(self.focused_pane_idx);
                return;
            }
        }
    }

    pub async fn handle_chat_list_click(&mut self, y: u16, list_area: Rect) -> Result<()> {
        let border_offset = if self.show_borders { 1 } else { 0 };
        if y < list_area.y + border_offset || y >= list_area.y + list_area.height - border_offset {
            return Ok(());
        }
        
        let relative_y = (y - list_area.y - border_offset) as usize;
        let ordered_chats = self.chat_list_order();
        let (unread_group, active_group, muted_group, other_group) = self.chat_list_groups();

        let mut row_map: Vec<Option<usize>> = Vec::new();
        let mut ordered_idx = 0usize;
        if !unread_group.is_empty() {
            row_map.push(None);
            for _ in unread_group.iter() {
                row_map.push(Some(ordered_idx));
                ordered_idx += 1;
            }
        }
        if !active_group.is_empty() {
            row_map.push(None);
            for _ in active_group.iter() {
                row_map.push(Some(ordered_idx));
                ordered_idx += 1;
            }
        }
        if !other_group.is_empty() {
            row_map.push(None);
            for _ in other_group.iter() {
                row_map.push(Some(ordered_idx));
                ordered_idx += 1;
            }
        }
        if !muted_group.is_empty() {
            row_map.push(None);
            for _ in muted_group.iter() {
                row_map.push(Some(ordered_idx));
                ordered_idx += 1;
            }
        }

        if relative_y < row_map.len() {
            if let Some(chat_idx) = row_map[relative_y].and_then(|idx| ordered_chats.get(idx).copied()) {
                let chat = &self.chats[chat_idx];
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
                    pane.messages.clear();
                    pane.reply_to_message = None;
                    pane.hide_reply_preview();
                    pane.scroll_offset = 0;

                    if let Some(chat_info) = self.chats.iter_mut().find(|c| c.id == chat_id) {
                        chat_info.unread = 0;
                    }
                }
                if let Some(list_idx) = row_map[relative_y] {
                    self.selected_chat_idx = list_idx;
                }
            }
        }
        Ok(())
    }

    fn refresh_all_pane_displays(&mut self) {
        for pane in &mut self.panes {
            pane.format_cache.clear();
        }
    }


    pub fn handle_up(&mut self) {
        if self.focus_on_chat_list {
            let max_idx = self.chat_list_order().len().saturating_sub(1);
            if self.selected_chat_idx > max_idx {
                self.selected_chat_idx = max_idx;
            } else if self.selected_chat_idx > 0 {
                self.selected_chat_idx -= 1;
            }
        } else {
            if !self.input_history.is_empty() {
                if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
                    match self.history_idx {
                        None => {
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
            let max_idx = self.chat_list_order().len().saturating_sub(1);
            if self.selected_chat_idx < max_idx {
                self.selected_chat_idx += 1;
            }
        } else {
            if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
                if let Some(idx) = self.history_idx {
                    if idx + 1 < self.input_history.len() {
                        self.history_idx = Some(idx + 1);
                        pane.input_buffer = self.input_history[idx + 1].clone();
                        pane.input_cursor = pane.input_buffer.len();
                    } else {
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

    pub fn handle_tab(&mut self) {
        let is_empty = self.panes.get(self.focused_pane_idx)
            .map_or(true, |p| p.input_buffer.is_empty());
        
        if is_empty {
            self.cycle_focus();
            return;
        }

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
                let ordered_chats = self.chat_list_order();
                if let Some(&chat_idx) = ordered_chats.get(self.selected_chat_idx) {
                    let chat = &self.chats[chat_idx];
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
                        pane.messages.clear();
                        pane.reply_to_message = None;
                        pane.hide_reply_preview();
                        pane.scroll_offset = 0;

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
            let (input_text, _chat_id, _reply_to_id) = if let Some(pane) = self.panes.get(self.focused_pane_idx) {
                (pane.input_buffer.clone(), pane.chat_id, pane.reply_to_message)
            } else {
                return Ok(());
            };

            if self.input_history.last().map_or(true, |last| last != &input_text) {
                self.input_history.push(input_text.clone());
                if self.input_history.len() > 100 {
                    self.input_history.remove(0);
                }
            }
            self.history_idx = None;
            self.history_temp.clear();

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

            if let Some(pane) = self.panes.get_mut(self.focused_pane_idx) {
                if let (Some(chat_id), Some(reply_to_id)) =
                    (pane.chat_id, pane.reply_to_message)
                {
                    let new_msg = crate::widgets::MessageData {
                        msg_id: 0,
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
                    
                    let telegram = self.telegram.clone();
                    let chat_id_copy = chat_id;
                    let reply_to_id_copy = reply_to_id;
                    let input_text_copy = input_text.clone();
                    tokio::spawn(async move {
                        let _ = telegram.reply_to_message(chat_id_copy, reply_to_id_copy, &input_text_copy).await;
                    });
                } else if let Some(chat_id) = pane.chat_id {
                    let new_msg = crate::widgets::MessageData {
                        msg_id: 0,
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


    pub async fn process_telegram_events(&mut self) -> Result<bool> {
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
                    let normalized_id = crate::utils::normalize_chat_id(chat_id);

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
                                pane.format_cache.clear();
                            }
                        }
                    }
                } else {
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

                            let is_muted = self.muted_chat_ids.contains(&chat_info.id);
                            if self.show_notifications && !is_outgoing && !is_muted {
                                send_desktop_notification(&chat_name, &preview);
                            }

                            if !is_muted {
                                self.notify(&format!("{}: {}", chat_name, preview));
                            }
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


    pub fn save_state(&self) -> Result<()> {
        let mut muted_chat_ids: Vec<i64> = self.muted_chat_ids.iter().copied().collect();
        muted_chat_ids.sort_unstable();
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
            muted_chat_ids,
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
        config.settings.chat_list_width = self.chat_list_width;
        config.settings.show_unread_count = self.show_unread_count;
        config.save()?;

        Ok(())
    }
}

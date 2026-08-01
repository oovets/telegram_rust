use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap},
};

use crate::commands::CommandHandler;
use crate::config::Config;
use crate::formatting::format_messages_for_display;
use crate::kitty_preview;
use crate::persistence::{Aliases, AppState, LayoutData, PaneState};
use crate::split_view::{PaneNode, SplitDirection};
use crate::telegram::TelegramClient;
use crate::utils::{send_desktop_notification, try_autocomplete};
use crate::widgets::{ChatPane, MessageData, message_data_from_raw, message_data_from_search};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A pending delete awaiting confirmation
#[derive(Clone)]
pub struct DeletePending {
    pub pane_idx: usize,
    pub chat_id: i64,
    pub chat_name: String,
    pub msg_num: i32,
    pub actual_msg_id: i32,
}

/// Result of a background operation, delivered via an unbounded channel
pub enum OpResult {
    ChatOpened {
        pane_idx: usize,
        chat_id: i64,
        chat_name: String,
        username: Option<String>,
        msgs: Vec<MessageData>,
        unread: u32,
        err: Option<String>,
    },
    ChatsRefreshed {
        chats: Vec<ChatInfo>,
        err: Option<String>,
    },
    SearchDone {
        pane_idx: usize,
        query: String,
        msgs: Vec<MessageData>,
        err: Option<String>,
    },
    MediaReady {
        pane_idx: usize,
        chat_id: i64,
        msg_id: i32,
        msg_num: i32,
        path: std::result::Result<String, String>,
    },
    MembersReady {
        pane_idx: usize,
        result: std::result::Result<Vec<(i64, String, String)>, String>,
    },
    GroupCreated {
        pane_idx: usize,
        name: String,
        chat_id: i64,
        msgs: Vec<MessageData>,
        chats: Vec<ChatInfo>,
        err: Option<String>,
    },
    UserResolved {
        pane_idx: usize,
        username: String,
        result: std::result::Result<(i64, String, Option<String>, Vec<MessageData>), String>,
    },
    AliasResolved {
        pane_idx: usize,
        target: Option<String>,
        result: std::result::Result<Option<i64>, String>,
    },
    PaneReload {
        panes: Vec<usize>,
        msgs: Vec<MessageData>,
        err: Option<String>,
    },
    ActionDone {
        pane_idx: usize,
        message: String,
        success: bool,
    },
}

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
    pub chat_list_state: ListState,
    pub show_help: bool,
    pub status_color: Color,
    pub pending_delete: Option<DeletePending>,
    pub pending_ops: Vec<tokio::sync::mpsc::UnboundedReceiver<OpResult>>,
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
        let total_panes_needed = (max_required_idx + 1)
            .max(app_state.layout.panes.len())
            .max(1);

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
            chat_list_state: ListState::default(),
            show_help: false,
            status_color: Color::Yellow,
            pending_delete: None,
            pending_ops: Vec::new(),
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

    fn spawn_op<F>(&mut self, op: F)
    where
        F: std::future::Future<Output = OpResult> + Send + 'static,
    {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<OpResult>();
        self.pending_ops.push(rx);
        tokio::spawn(async move {
            let result = op.await;
            let _ = tx.send(result);
        });
    }

    /// Poll all pending background operations and apply their results.
    pub fn drain_pending_ops(&mut self) {
        let mut to_apply: Vec<OpResult> = Vec::new();
        self.pending_ops.retain_mut(|rx| match rx.try_recv() {
            Ok(result) => {
                to_apply.push(result);
                true
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        if !to_apply.is_empty() {
            for result in to_apply {
                self.apply_op_result(result);
            }
            self.needs_redraw = true;
        }
    }

    fn apply_op_result(&mut self, result: OpResult) {
        match result {
            OpResult::ChatOpened {
                pane_idx,
                chat_id,
                chat_name,
                username,
                msgs,
                unread,
                err,
            } => {
                if let Some(err) = err {
                    self.notify_error(&err);
                    if let Some(pane) = self.panes.get_mut(pane_idx) {
                        pane.loading = false;
                    }
                    return;
                }
                self.apply_chat_open(pane_idx, chat_id, chat_name, username, msgs, unread);
            }
            OpResult::ChatsRefreshed { chats, err } => {
                if let Some(err) = err {
                    self.notify_error(&err);
                    return;
                }
                self.chats = chats;
            }
            OpResult::SearchDone {
                pane_idx,
                query,
                msgs,
                err,
            } => {
                if let Some(pane) = self.panes.get_mut(pane_idx) {
                    pane.loading = false;
                }
                if let Some(err) = err {
                    self.notify_error(&err);
                    return;
                }
                self.apply_search(pane_idx, &query, msgs);
            }
            OpResult::MediaReady {
                pane_idx,
                chat_id,
                msg_id,
                msg_num,
                path,
            } => {
                if let Some(pane) = self.panes.get_mut(pane_idx) {
                    pane.loading = false;
                }
                match path {
                    Ok(path) => {
                        self.handle_media_downloaded(pane_idx, chat_id, msg_id, msg_num, &path)
                    }
                    Err(e) => self.notify_error(&format!("Download failed: {}", e)),
                }
            }
            OpResult::MembersReady { pane_idx, result } => {
                if let Some(pane) = self.panes.get_mut(pane_idx) {
                    pane.loading = false;
                }
                match result {
                    Ok(members) => {
                        if let Some(pane) = self.panes.get_mut(pane_idx) {
                            pane.add_message(format!("--- Members ({}) ---", members.len()));
                            for (id, name, role) in &members {
                                pane.add_message(format!("  {} (id:{}) - {}", name, id, role));
                            }
                            pane.add_message("---".to_string());
                        }
                        self.notify_success(&format!("{} members", members.len()));
                    }
                    Err(e) => self.notify_error(&format!("Failed to load members: {}", e)),
                }
            }
            OpResult::GroupCreated {
                pane_idx,
                name,
                chat_id,
                msgs,
                chats,
                err,
            } => {
                if let Some(pane) = self.panes.get_mut(pane_idx) {
                    pane.loading = false;
                }
                if let Some(err) = err {
                    self.notify_error(&format!("Failed to create group: {}", err));
                    return;
                }
                self.chats = chats;
                self.apply_chat_open(pane_idx, chat_id, name.clone(), None, msgs, 0);
                self.notify_success(&format!("Group '{}' created", name));
            }
            OpResult::UserResolved {
                pane_idx,
                username,
                result,
            } => {
                if let Some(pane) = self.panes.get_mut(pane_idx) {
                    pane.loading = false;
                }
                match result {
                    Ok((chat_id, chat_name, chat_username, msgs)) => {
                        self.apply_chat_open(pane_idx, chat_id, chat_name, chat_username, msgs, 0);
                    }
                    Err(e) => self.notify_error(&format!("{}: {}", username, e)),
                }
            }
            OpResult::AliasResolved {
                pane_idx,
                target,
                result,
            } => match result {
                Ok(Some(sender_id)) => {
                    if let Some(alias) = target {
                        self.aliases.insert(sender_id, alias.clone());
                        match self.aliases.save(&self.config) {
                            Ok(_) => {
                                if let Some(pane) = self.panes.get_mut(pane_idx) {
                                    pane.add_message(format!("✓ Alias set: {}", alias));
                                }
                                self.notify_success(&format!("Alias set: {}", alias));
                            }
                            Err(e) => self.notify_error(&format!("Failed to save alias: {}", e)),
                        }
                    } else {
                        match self.aliases.remove(&sender_id) {
                            Some(_) => {
                                if let Some(pane) = self.panes.get_mut(pane_idx) {
                                    pane.add_message("✓ Alias removed".to_string());
                                }
                                self.notify_success("Alias removed");
                            }
                            None => self.notify_error("No alias set for this user"),
                        }
                    }
                }
                Ok(None) => self.notify_error("Could not find message sender"),
                Err(e) => self.notify_error(&format!("Lookup failed: {}", e)),
            },
            OpResult::PaneReload { panes, msgs, err } => {
                if let Some(err) = err {
                    let _ = err;
                    return;
                }
                for pane_idx in panes {
                    if let Some(pane) = self.panes.get_mut(pane_idx) {
                        if pane.loading {
                            continue;
                        }
                        pane.msg_data = msgs.clone();
                        pane.format_cache.clear();
                    }
                }
            }
            OpResult::ActionDone {
                pane_idx,
                message,
                success,
            } => {
                if success {
                    if let Some(pane) = self.panes.get_mut(pane_idx) {
                        pane.add_message(format!("✓ {}", message));
                    }
                    self.notify_success(&message);
                } else {
                    if let Some(pane) = self.panes.get_mut(pane_idx) {
                        pane.add_message(format!("✗ {}", message));
                    }
                    self.notify_error(&message);
                }
            }
        }
    }

    /// Open a chat in a pane without blocking the UI (background fetch)
    pub fn queue_open_chat(
        &mut self,
        pane_idx: usize,
        chat_id: i64,
        chat_name: String,
        username: Option<String>,
        unread: u32,
    ) {
        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.loading = true;
        }
        let telegram = self.telegram.clone();
        let my_user_id = self.my_user_id;
        self.spawn_op(async move {
            match telegram.get_messages(chat_id, 50).await {
                Ok(raw) => OpResult::ChatOpened {
                    pane_idx,
                    chat_id,
                    chat_name,
                    username,
                    msgs: message_data_from_raw(&raw, my_user_id),
                    unread,
                    err: None,
                },
                Err(e) => OpResult::ChatOpened {
                    pane_idx,
                    chat_id,
                    chat_name,
                    username,
                    msgs: Vec::new(),
                    unread,
                    err: Some(format!("Failed to load chat: {}", e)),
                },
            }
        });
    }

    /// Reload messages for a pane only if it is still empty (mouse-click lazy load)
    pub fn queue_pane_reload_if_needed(&mut self, pane_idx: usize) {
        if let Some(pane) = self.panes.get(pane_idx)
            && let Some(chat_id) = pane.chat_id
            && pane.msg_data.is_empty()
            && !pane.loading
        {
            let telegram = self.telegram.clone();
            let my_user_id = self.my_user_id;
            self.spawn_op(async move {
                match telegram.get_messages(chat_id, 50).await {
                    Ok(raw) => OpResult::PaneReload {
                        panes: vec![pane_idx],
                        msgs: message_data_from_raw(&raw, my_user_id),
                        err: None,
                    },
                    Err(e) => OpResult::PaneReload {
                        panes: vec![pane_idx],
                        msgs: Vec::new(),
                        err: Some(e.to_string()),
                    },
                }
            });
        }
    }

    /// Refresh the chat list in the background (Ctrl+R)
    pub fn queue_refresh_chats(&mut self) {
        let telegram = self.telegram.clone();
        self.spawn_op(async move {
            match telegram.get_dialogs().await {
                Ok(chats) => OpResult::ChatsRefreshed { chats, err: None },
                Err(e) => OpResult::ChatsRefreshed {
                    chats: Vec::new(),
                    err: Some(format!("Refresh failed: {}", e)),
                },
            }
        });
    }

    pub fn queue_search(&mut self, pane_idx: usize, chat_id: i64, query: String) {
        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.loading = true;
        }
        let telegram = self.telegram.clone();
        let my_user_id = self.my_user_id;
        self.spawn_op(async move {
            match telegram.search_messages(chat_id, &query, 100).await {
                Ok(raw) => OpResult::SearchDone {
                    pane_idx,
                    query,
                    msgs: message_data_from_search(&raw, my_user_id),
                    err: None,
                },
                Err(e) => OpResult::SearchDone {
                    pane_idx,
                    query,
                    msgs: Vec::new(),
                    err: Some(format!("Search failed: {}", e)),
                },
            }
        });
    }

    pub fn queue_media_download(
        &mut self,
        pane_idx: usize,
        chat_id: i64,
        msg_id: i32,
        msg_num: i32,
    ) {
        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.loading = true;
        }
        let telegram = self.telegram.clone();
        let downloads_dir = std::env::temp_dir();
        self.spawn_op(async move {
            match telegram
                .download_media_by_id(chat_id, msg_id, &downloads_dir)
                .await
            {
                Ok(path) => OpResult::MediaReady {
                    pane_idx,
                    chat_id,
                    msg_id,
                    msg_num,
                    path: Ok(path),
                },
                Err(e) => OpResult::MediaReady {
                    pane_idx,
                    chat_id,
                    msg_id,
                    msg_num,
                    path: Err(e.to_string()),
                },
            }
        });
    }

    pub fn queue_resolve_and_open(&mut self, pane_idx: usize, username: String) {
        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.loading = true;
        }
        let telegram = self.telegram.clone();
        let my_user_id = self.my_user_id;
        self.spawn_op(async move {
            match telegram.resolve_username(&username).await {
                Ok(Some((chat_id, chat_name, _is_group))) => {
                    match telegram.get_messages(chat_id, 50).await {
                        Ok(raw) => OpResult::UserResolved {
                            pane_idx,
                            username: username.clone(),
                            result: Ok((
                                chat_id,
                                chat_name,
                                Some(format!("@{}", username.trim_start_matches('@'))),
                                message_data_from_raw(&raw, my_user_id),
                            )),
                        },
                        Err(e) => OpResult::UserResolved {
                            pane_idx,
                            username: username.clone(),
                            result: Err(format!("Failed to load messages: {}", e)),
                        },
                    }
                }
                Ok(None) => OpResult::UserResolved {
                    pane_idx,
                    username: username.clone(),
                    result: Err(format!("User '{}' not found", username)),
                },
                Err(e) => OpResult::UserResolved {
                    pane_idx,
                    username,
                    result: Err(e.to_string()),
                },
            }
        });
    }

    pub fn queue_create_group(&mut self, pane_idx: usize, name: String) {
        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.loading = true;
        }
        let telegram = self.telegram.clone();
        let my_user_id = self.my_user_id;
        self.spawn_op(async move {
            match telegram.create_group(&name, vec![]).await {
                Ok(chat_id) => {
                    let msgs = match telegram.get_messages(chat_id, 50).await {
                        Ok(raw) => message_data_from_raw(&raw, my_user_id),
                        Err(_) => Vec::new(),
                    };
                    let chats = telegram.get_dialogs().await.unwrap_or_default();
                    OpResult::GroupCreated {
                        pane_idx,
                        name,
                        chat_id,
                        msgs,
                        chats,
                        err: None,
                    }
                }
                Err(e) => OpResult::GroupCreated {
                    pane_idx,
                    name,
                    chat_id: 0,
                    msgs: Vec::new(),
                    chats: Vec::new(),
                    err: Some(e.to_string()),
                },
            }
        });
    }

    pub fn queue_members(&mut self, pane_idx: usize, chat_id: i64) {
        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.loading = true;
        }
        let telegram = self.telegram.clone();
        self.spawn_op(async move {
            let result = telegram
                .get_members(chat_id)
                .await
                .map_err(|e| e.to_string());
            OpResult::MembersReady { pane_idx, result }
        });
    }

    pub fn queue_add_remove(
        &mut self,
        pane_idx: usize,
        action: String,
        target: String,
        chat_id: i64,
    ) {
        let telegram = self.telegram.clone();
        self.spawn_op(async move {
            let result = match action.as_str() {
                "add" => telegram.add_member(chat_id, &target).await,
                _ => telegram.remove_member(chat_id, &target).await,
            };
            let (success, message) = match result {
                Ok(_) => {
                    let verb = if action == "add" { "Added" } else { "Removed" };
                    (
                        true,
                        format!(
                            "{} {} {}",
                            verb,
                            target,
                            if action == "add" {
                                "to group"
                            } else {
                                "from group"
                            }
                        ),
                    )
                }
                Err(e) => (
                    false,
                    format!(
                        "{} failed: {}",
                        if action == "add" { "Add" } else { "Remove" },
                        e
                    ),
                ),
            };
            OpResult::ActionDone {
                pane_idx,
                message,
                success,
            }
        });
    }

    pub fn queue_edit_message(
        &mut self,
        pane_idx: usize,
        chat_id: i64,
        msg_id: i32,
        msg_num: i32,
        new_text: String,
    ) {
        let telegram = self.telegram.clone();
        self.spawn_op(async move {
            let result = telegram.edit_message(chat_id, msg_id, &new_text).await;
            let (success, message) = match result {
                Ok(_) => (true, format!("Edited message #{}", msg_num)),
                Err(e) => (false, format!("Edit failed: {}", e)),
            };
            OpResult::ActionDone {
                pane_idx,
                message,
                success,
            }
        });
    }

    pub fn queue_delete_message(
        &mut self,
        pane_idx: usize,
        chat_id: i64,
        msg_id: i32,
        msg_num: i32,
    ) {
        let telegram = self.telegram.clone();
        self.spawn_op(async move {
            let result = telegram.delete_message(chat_id, msg_id).await;
            let (success, message) = match result {
                Ok(_) => (true, format!("Deleted message #{}", msg_num)),
                Err(e) => (false, format!("Delete failed: {}", e)),
            };
            OpResult::ActionDone {
                pane_idx,
                message,
                success,
            }
        });
    }

    pub fn queue_reply_message(
        &mut self,
        pane_idx: usize,
        chat_id: i64,
        msg_id: i32,
        msg_num: i32,
        text: String,
    ) {
        let telegram = self.telegram.clone();
        self.spawn_op(async move {
            let result = telegram.reply_to_message(chat_id, msg_id, &text).await;
            let (success, message) = match result {
                Ok(_) => (true, format!("Replied to #{}", msg_num)),
                Err(e) => (false, format!("Reply failed: {}", e)),
            };
            OpResult::ActionDone {
                pane_idx,
                message,
                success,
            }
        });
    }

    pub fn queue_forward(
        &mut self,
        pane_idx: usize,
        msg_num: i32,
        target: String,
        from_chat_id: i64,
        message_id: i32,
    ) {
        let telegram = self.telegram.clone();
        self.spawn_op(async move {
            let result = async {
                let (to_chat_id, _name, _is_group) = telegram
                    .resolve_username(&target)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("User '{}' not found", target))?;
                telegram
                    .forward_message(from_chat_id, message_id, to_chat_id)
                    .await
            }
            .await;
            let (success, message) = match result {
                Ok(_) => (true, format!("Forwarded #{} to {}", msg_num, target)),
                Err(e) => (false, format!("Forward failed: {}", e)),
            };
            OpResult::ActionDone {
                pane_idx,
                message,
                success,
            }
        });
    }

    pub fn queue_resolve_sender(
        &mut self,
        pane_idx: usize,
        chat_id: i64,
        msg_id: i32,
        target: Option<String>,
    ) {
        let telegram = self.telegram.clone();
        self.spawn_op(async move {
            let result = telegram
                .get_message_sender(chat_id, msg_id)
                .await
                .map_err(|e| e.to_string());
            OpResult::AliasResolved {
                pane_idx,
                target,
                result,
            }
        });
    }

    fn apply_chat_open(
        &mut self,
        pane_idx: usize,
        chat_id: i64,
        chat_name: String,
        username: Option<String>,
        msgs: Vec<MessageData>,
        unread: u32,
    ) {
        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.loading = false;
            pane.chat_id = Some(chat_id);
            pane.chat_name = chat_name;
            pane.username = username;
            pane.msg_data = msgs;
            pane.messages.clear();
            pane.reply_to_message = None;
            pane.hide_reply_preview();
            pane.scroll_offset = 0;
            pane.format_cache.clear();
            pane.unread_count_at_load = unread;
        }
        if let Some(chat_info) = self.chats.iter_mut().find(|c| c.id == chat_id) {
            chat_info.unread = 0;
        }
    }

    fn apply_search(&mut self, pane_idx: usize, query: &str, msgs: Vec<MessageData>) {
        if let Some(pane) = self.panes.get_mut(pane_idx) {
            if !pane.search_active {
                pane.search_active = true;
                pane.saved_chat_name = Some(pane.chat_name.clone());
                pane.saved_msg_data = Some(std::mem::take(&mut pane.msg_data));
            }
            let base = pane
                .chat_name
                .split(" | Search:")
                .next()
                .unwrap_or(&pane.chat_name)
                .to_string();
            pane.chat_name = format!("{} | Search: '{}'", base, query);
            pane.msg_data = msgs;
            pane.scroll_offset = 0;
        }
        self.notify_success(&format!("Search: '{}'", query));
    }

    /// Exit search mode and restore the normal chat view
    pub fn restore_search(&mut self, pane_idx: usize) {
        if let Some(pane) = self.panes.get_mut(pane_idx).filter(|p| p.search_active) {
            if let Some(saved) = pane.saved_msg_data.take() {
                pane.msg_data = saved;
            }
            if let Some(saved_name) = pane.saved_chat_name.take() {
                pane.chat_name = saved_name;
            }
            pane.search_active = false;
            pane.scroll_offset = 0;
            pane.format_cache.clear();
            self.notify("Search cleared");
        }
    }

    fn handle_media_downloaded(
        &mut self,
        pane_idx: usize,
        chat_id: i64,
        msg_id: i32,
        msg_num: i32,
        path: &str,
    ) {
        let is_image = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                matches!(
                    e.to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif"
                )
            })
            .unwrap_or(false);

        if is_image && kitty_preview::supports_kitty_graphics() {
            let preview_path = if path.to_ascii_lowercase().ends_with(".png") {
                path.to_string()
            } else {
                match image::open(path) {
                    Ok(img) => {
                        let png_path = std::env::temp_dir()
                            .join(format!("telegram_preview_{}_{}.png", chat_id, msg_id));
                        match img.save_with_format(&png_path, image::ImageFormat::Png) {
                            Ok(_) => png_path.to_string_lossy().to_string(),
                            Err(e) => {
                                self.notify_error(&format!("Preview conversion failed: {}", e));
                                path.to_string()
                            }
                        }
                    }
                    Err(e) => {
                        self.notify_error(&format!("Preview decode failed: {}", e));
                        path.to_string()
                    }
                }
            };
            self.open_inline_preview_for_message(pane_idx, chat_id, msg_id, preview_path);
            self.notify_with_duration("Inline preview opened (Esc to close)", 3);
        } else {
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("open").arg(path).spawn();
            }
            #[cfg(target_os = "linux")]
            {
                let _ = std::process::Command::new("xdg-open").arg(path).spawn();
            }
            self.notify_success(&format!(
                "✓ {}",
                std::path::Path::new(path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ));
        }
        let _ = msg_num;
    }

    async fn load_saved_chat_messages(&mut self) -> Result<()> {
        for pane in self.panes.iter_mut() {
            if let Some(chat_id) = pane.chat_id
                && let Ok(raw_messages) = self.telegram.get_messages(chat_id, 50).await
                && !raw_messages.is_empty()
            {
                pane.msg_data = message_data_from_raw(&raw_messages, self.my_user_id);
                pane.format_cache.clear();

                if let Some(chat_info) = self.chats.iter().find(|c| c.id == chat_id) {
                    pane.username = chat_info.username.clone();
                }
            }
        }
        Ok(())
    }

    pub fn draw(&mut self, f: &mut Frame) {
        for pane in &mut self.panes {
            pane.check_typing_expired();
        }
        if self
            .status_expire
            .is_some_and(|e| std::time::Instant::now() >= e)
        {
            self.status_message = None;
            self.status_expire = None;
        }

        // Mode indicator chips for the focused pane
        let mut chips: Vec<String> = Vec::new();
        let focused_pane = self.panes.get(self.focused_pane_idx);
        if !self.focus_on_chat_list
            && let Some(pane) = focused_pane
        {
            if pane.loading {
                chips.push("LOADING".to_string());
            }
            if pane.search_active {
                chips.push("SEARCH".to_string());
            }
            if let Some(ft) = &pane.filter_type {
                let name = match ft {
                    crate::widgets::FilterType::Sender => "sender",
                    crate::widgets::FilterType::Media => "media",
                    crate::widgets::FilterType::Link => "link",
                };
                chips.push(format!("FILTER:{}", name));
            }
            if pane.reply_to_message.is_some() {
                chips.push("REPLY".to_string());
            }
        }

        // Status bar is always one row: [chips] [status message] [pending delete]
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
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
            Color::Cyan,
            Color::Yellow,
            Color::Magenta,
            Color::Blue,
            Color::Red,
            Color::Green,
            Color::White,
            Color::LightCyan,
            Color::LightYellow,
            Color::LightMagenta,
            Color::LightBlue,
            Color::LightRed,
            Color::LightGreen,
            Color::DarkGray,
            Color::Rgb(192, 192, 192),
            Color::Rgb(255, 165, 0),
            Color::Rgb(255, 192, 203),
            Color::Rgb(128, 0, 128),
            Color::Rgb(0, 255, 255),
            Color::Rgb(255, 20, 147),
        ];

        let mut senders_to_color: Vec<i64> = Vec::new();
        for pane in &self.panes {
            if let Some(chat_id) = pane.chat_id {
                let is_group_chat = self.chats.iter().any(|c| c.id == chat_id && c.is_group);
                if is_group_chat && !pane.msg_data.is_empty() {
                    for msg in &pane.msg_data {
                        if !self.user_colors.contains_key(&msg.sender_id)
                            && !senders_to_color.contains(&msg.sender_id)
                        {
                            senders_to_color.push(msg.sender_id);
                        }
                    }
                }
            }
        }

        for &sender_id in &senders_to_color {
            let mut hash = sender_id.unsigned_abs();
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
        self.pane_tree.render(
            f,
            pane_area,
            &self.panes,
            self.focused_pane_idx,
            &render_fn,
            &mut pane_areas,
        );
        self.pane_areas = pane_areas;

        // Inline preview: drawn as an overlay on the right so the pane
        // layout does not jump when it opens/closes.
        if self.inline_preview_path.is_some() {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(pane_area);
            let preview_panel = split[1];
            let preview_title = self
                .inline_preview_name
                .as_deref()
                .unwrap_or("Inline image preview");
            let zoom = self.inline_preview_zoom_pct;
            let idx_info = if let (Some(i), total) = (
                self.inline_preview_index,
                self.inline_preview_image_msg_ids.len(),
            ) {
                if total > 0 {
                    format!(" [{}/{}]", i + 1, total)
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            f.render_widget(Clear, preview_panel);
            let block = Block::default().borders(Borders::ALL).title(format!(
                "Preview: {} ({}%){}",
                preview_title, zoom, idx_info
            ));
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

        // Status bar
        let mut status_spans: Vec<Span> = Vec::new();
        for chip in &chips {
            status_spans.push(Span::styled(
                format!("[{}] ", chip),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        if let Some(ref msg) = self.status_message {
            status_spans.push(Span::styled(
                msg.clone(),
                Style::default()
                    .fg(self.status_color)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        if let Some(ref pd) = self.pending_delete {
            status_spans.push(Span::styled(
                format!(
                    "Delete #{} from '{}'?  [y]es  [n]/Esc cancel",
                    pd.msg_num, pd.chat_name
                ),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
        }
        f.render_widget(Paragraph::new(Line::from(status_spans)), outer[1]);

        if self.show_help {
            self.draw_help_overlay(f);
        }
    }

    fn draw_help_overlay(&self, f: &mut Frame) {
        let help_lines: Vec<&str> = vec![
            "Global:      Ctrl+Q quit  ·  Ctrl+R refresh  ·  ? / Ctrl+H help",
            "             Tab / Shift+Tab cycle focus  ·  Alt+Left/Right prev/next pane",
            "             Ctrl+Left/Right resize sidebar  ·  Esc cancel",
            "",
            "Panes:       Ctrl+V split vertical  ·  Ctrl+B split horizontal  ·  Ctrl+K toggle direction",
            "             Ctrl+W close pane  ·  Ctrl+L clear  ·  PageUp/PageDown scroll",
            "",
            "Chat list:   Up/Down navigate  ·  Enter open  ·  Ctrl+P mute/unmute  ·  Ctrl+S toggle",
            "",
            "Display:     Ctrl+E reactions  ·  Ctrl+N notifications  ·  Ctrl+D compact  ·  Ctrl+O emojis",
            "             Ctrl+G line numbers  ·  Ctrl+T timestamps  ·  Ctrl+M unread count",
            "             Ctrl+U user colors  ·  Ctrl+Y borders",
            "",
            "Inline preview:  Esc close  ·  +/- zoom  ·  n/p or Left/Right next/prev image",
            "",
            "Commands:    /reply  /media  /edit  /delete  /search  /filter  /alias",
            "             /new  /newgroup  /add  /kick  /members  /forward",
            "",
            "Any key closes this help",
        ];
        let width = help_lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16 + 4;
        let height = help_lines.len() as u16 + 2;
        let x = f
            .area()
            .width
            .saturating_sub(width)
            .saturating_div(2)
            .max(1);
        let y = f
            .area()
            .height
            .saturating_sub(height)
            .saturating_div(2)
            .max(1);
        let rect = Rect {
            x,
            y,
            width,
            height,
        };
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Help - telegram_client_rs ")
            .border_style(Style::default().fg(Color::Green));
        let para = Paragraph::new(help_lines.join("\n"))
            .block(block)
            .style(Style::default().fg(Color::White));
        f.render_widget(para, rect);
    }

    fn draw_chat_list(&mut self, f: &mut Frame, area: Rect) {
        let active_chat_id = self
            .panes
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
            if let Some(username) = chat.username.as_ref().filter(|u| !u.is_empty()) {
                name_part.push_str(&format!(" {}", username));
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

            let total_chars: usize = spans.iter().map(|s| s.content.as_ref().width()).sum();
            let truncated = total_chars > max_width && max_width > 0;
            let mut remaining = if truncated {
                max_width.saturating_sub(1)
            } else {
                max_width
            };
            let mut out_spans: Vec<ratatui::text::Span> = Vec::new();

            for span in spans.into_iter() {
                if remaining == 0 {
                    break;
                }
                let span_len = span.content.as_ref().width();
                if span_len <= remaining {
                    remaining = remaining.saturating_sub(span_len);
                    out_spans.push(span);
                } else {
                    let mut clipped = String::new();
                    let mut clipped_w = 0usize;
                    for c in span.content.chars() {
                        let cw = UnicodeWidthChar::width(c).unwrap_or(1).max(1);
                        if clipped_w + cw > remaining {
                            break;
                        }
                        clipped_w += cw;
                        clipped.push(c);
                    }
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

        // Track which row in the rendered list corresponds to the selection
        // (ordered index into chats, skipping group header rows)
        let ordered = self.chat_list_order();
        if ordered.is_empty() {
            self.selected_chat_idx = 0;
        } else if self.selected_chat_idx >= ordered.len() {
            self.selected_chat_idx = ordered.len() - 1;
        }
        let mut ordered_idx = 0usize;
        let mut selected_row: Option<usize> = None;
        let groups = [
            (&unread_group, "Unread"),
            (&active_group, "Active"),
            (&other_group, "Other"),
            (&muted_group, "Muted"),
        ];
        for (group, title) in groups {
            if group.is_empty() {
                continue;
            }
            items.push(ListItem::new(title).style(header_style));
            for chat_idx in group.iter() {
                if ordered_idx == self.selected_chat_idx {
                    selected_row = Some(items.len());
                }
                items.push(build_item(&self.chats[*chat_idx]));
                ordered_idx += 1;
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
        self.chat_list_state.select(selected_row);
        let list = List::new(items)
            .block(list_block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▌ ");

        f.render_stateful_widget(list, area, &mut self.chat_list_state);
    }

    fn draw_chat_pane_impl(&self, f: &mut Frame, area: Rect, pane: &ChatPane, is_focused: bool) {
        let has_reply_preview = pane.reply_preview.is_some();

        let border_overhead = if self.show_borders { 2 } else { 0 };
        let header_height = if self.show_borders { 3 } else { 1 };
        let inner_width = area
            .width
            .saturating_sub(if self.show_borders { 2 } else { 0 })
            .max(1) as usize;
        let text_lines = if is_focused && inner_width > 0 {
            let buf = &pane.input_buffer;
            let mut lines: u16 = 0;
            for line in buf.split('\n') {
                let len = line.width();
                lines += ((len as f64) / (inner_width as f64)).ceil().max(1.0) as u16;
            }
            let last_line_len = buf.rsplit('\n').next().unwrap_or(buf).width() + 1;
            if last_line_len > inner_width {
                let without_cursor = buf.rsplit('\n').next().unwrap_or(buf).width();
                let lines_without = ((without_cursor as f64) / (inner_width as f64))
                    .ceil()
                    .max(1.0) as u16;
                let lines_with = ((last_line_len as f64) / (inner_width as f64))
                    .ceil()
                    .max(1.0) as u16;
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

        // Truncate the header instead of clipping it at the pane edge
        let header_max = chunks[0]
            .width
            .saturating_sub(if self.show_borders { 4 } else { 2 })
            .max(1) as usize;
        header_text = truncate_to_width(&header_text, header_max);

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

        let display_lines = if pane.loading && pane.msg_data.is_empty() {
            vec!["Loading...".to_string()]
        } else if !pane.msg_data.is_empty() {
            let filter_type = pane.filter_type.as_ref().map(|ft| match ft {
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
            if max_width == 0 || text.width() <= max_width {
                return vec![text.to_string()];
            }

            let mut lines = Vec::new();
            let mut current_line = String::new();
            let mut current_width = 0usize;

            for word in text.split_whitespace() {
                let word_width = word.width();
                if current_width + word_width + 1 > max_width {
                    if !current_line.is_empty() {
                        lines.push(std::mem::take(&mut current_line));
                        current_width = 0;
                    }
                    if word_width > max_width {
                        lines.extend(split_word_into_width_chunks(word, max_width));
                    } else {
                        current_line = word.to_string();
                        current_width = word_width;
                    }
                } else {
                    if !current_line.is_empty() {
                        current_line.push(' ');
                        current_width += 1;
                    }
                    current_line.push_str(word);
                    current_width += word_width;
                }
            }
            if !current_line.is_empty() {
                lines.push(current_line);
            }
            lines
        };

        let wrap_message_with_indent = |prefix: &str,
                                        sender_name: &str,
                                        message_text: &str,
                                        max_width: usize|
         -> Vec<String> {
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

                if msg == "Loading..." {
                    return vec![
                        Line::from(msg.as_str()).style(
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::ITALIC),
                        ),
                    ];
                }

                if let Some(rest) = msg.strip_prefix("[DAYSEP] ") {
                    return vec![
                        Line::from(rest.to_string()).style(
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ];
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
                                        self.user_colors
                                            .get(&sender_id)
                                            .copied()
                                            .unwrap_or(base_color)
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

        if let Some(preview) = pane.reply_preview.as_deref().filter(|_| has_reply_preview) {
            let reply_bar = Paragraph::new(preview).style(
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::ITALIC),
            );
            f.render_widget(reply_bar, chunks[2]);
        }

        let input_chunk = if has_reply_preview {
            chunks[3]
        } else {
            chunks[2]
        };
        let input_title = if is_focused && !self.focus_on_chat_list {
            "Input (Alt+Enter for newline, Tab to cycle)"
        } else {
            "Input"
        };
        let input_text = if is_focused {
            pane.input_buffer.clone()
        } else {
            String::new()
        };

        let input_block = if self.show_borders {
            Block::default().borders(Borders::ALL).title(input_title)
        } else {
            Block::default()
        };
        let input = Paragraph::new(input_text)
            .block(input_block)
            .wrap(Wrap { trim: false });
        f.render_widget(input, input_chunk);

        // Position the real terminal cursor on the input line
        if is_focused && !self.focus_on_chat_list {
            let border = if self.show_borders { 1 } else { 0 };
            let content_top = input_chunk.y + border;
            let content_bottom = input_chunk.y + input_chunk.height.saturating_sub(border);
            let content_rows = (content_bottom.saturating_sub(content_top)).max(1) as usize;
            let inner_w = (input_chunk
                .width
                .saturating_sub(if self.show_borders { 2 } else { 0 }))
            .max(1) as usize;
            let buf = &pane.input_buffer;
            let cursor_before = pane.input_cursor.min(buf.len());
            let before = &buf[..cursor_before];
            let total_lines = wrapped_line_count(buf, inner_w);
            let cursor_line = wrapped_line_count(before, inner_w).saturating_sub(1);
            let start = total_lines.saturating_sub(content_rows);
            let row = if cursor_line >= start {
                content_top + (cursor_line - start) as u16
            } else {
                content_bottom.saturating_sub(1)
            };
            let col = (before.width() % inner_w).min(inner_w.saturating_sub(1)) as u16;
            let x = input_chunk.x + border + col;
            f.set_cursor_position(Position { x, y: row });
        }
    }

    pub fn notify(&mut self, message: &str) {
        self.status_color = Color::Yellow;
        self.status_message = Some(message.to_string());
        self.status_expire = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
    }

    pub fn notify_with_duration(&mut self, message: &str, duration_secs: u64) {
        self.status_color = Color::Yellow;
        self.status_message = Some(message.to_string());
        self.status_expire =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(duration_secs));
    }

    pub fn notify_error(&mut self, message: &str) {
        self.status_color = Color::Red;
        self.status_message = Some(message.to_string());
        self.status_expire = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
    }

    pub fn notify_success(&mut self, message: &str) {
        self.status_color = Color::Green;
        self.status_message = Some(message.to_string());
        self.status_expire = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
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
                let png_path = std::env::temp_dir()
                    .join(format!("telegram_preview_{}_{}.png", chat_id, msg_id));
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

    pub fn load_pane_messages_if_needed(&mut self, pane_idx: usize) {
        self.queue_pane_reload_if_needed(pane_idx);
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

    fn split_pane_in_tree(&mut self, target_idx: usize, direction: SplitDirection, new_idx: usize) {
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
            PaneNode::Split {
                direction,
                children,
            } => {
                let is_direct_child = children.iter().any(
                    |child| matches!(child.as_ref(), PaneNode::Single(idx) if *idx == target_idx),
                );

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
            if let Some(current_pos) = all_panes
                .iter()
                .position(|&idx| idx == self.focused_pane_idx)
            {
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
            if let Some(current_pos) = all_panes
                .iter()
                .position(|&idx| idx == self.focused_pane_idx)
            {
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
        if let Some(current_pos) = all_panes
            .iter()
            .position(|&idx| idx == self.focused_pane_idx)
        {
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
        if let Some(current_pos) = all_panes
            .iter()
            .position(|&idx| idx == self.focused_pane_idx)
        {
            let prev = if current_pos > 0 {
                current_pos - 1
            } else {
                all_panes.len() - 1
            };
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
        let status = if self.show_notifications { "ON" } else { "OFF" };
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
        self.notify(&format!(
            "Chat list: {}",
            if self.show_chat_list { "ON" } else { "OFF" }
        ));
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
        self.notify(&format!(
            "Borders: {}",
            if self.show_borders { "ON" } else { "OFF" }
        ));
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
            self.panes
                .get(self.focused_pane_idx)
                .and_then(|p| p.chat_id)
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

    pub fn handle_chat_list_click(&mut self, y: u16, list_area: Rect) {
        let border_offset = if self.show_borders { 1 } else { 0 };
        if y < list_area.y + border_offset || y >= list_area.y + list_area.height - border_offset {
            return;
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

        if relative_y < row_map.len()
            && let Some(chat_idx) =
                row_map[relative_y].and_then(|idx| ordered_chats.get(idx).copied())
        {
            let chat = self.chats[chat_idx].clone();
            self.queue_open_chat(
                self.focused_pane_idx,
                chat.id,
                chat.name,
                chat.username,
                chat.unread,
            );
            if let Some(list_idx) = row_map[relative_y] {
                self.selected_chat_idx = list_idx;
            }
        }
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
                            pane.input_buffer =
                                self.input_history[self.input_history.len() - 1].clone();
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
        let is_empty = self
            .panes
            .get(self.focused_pane_idx)
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
        let input_empty = self
            .panes
            .get(self.focused_pane_idx)
            .map_or(true, |p| p.input_buffer.is_empty());

        if input_empty {
            if self.focus_on_chat_list && !self.chats.is_empty() {
                let ordered_chats = self.chat_list_order();
                if let Some(&chat_idx) = ordered_chats.get(self.selected_chat_idx) {
                    let chat = self.chats[chat_idx].clone();
                    self.queue_open_chat(
                        self.focused_pane_idx,
                        chat.id,
                        chat.name,
                        chat.username,
                        chat.unread,
                    );
                    self.focus_on_chat_list = false;
                }
            }
        } else if !self.focus_on_chat_list {
            let (input_text, _chat_id, _reply_to_id) =
                if let Some(pane) = self.panes.get(self.focused_pane_idx) {
                    (
                        pane.input_buffer.clone(),
                        pane.chat_id,
                        pane.reply_to_message,
                    )
                } else {
                    return Ok(());
                };

            if self
                .input_history
                .last()
                .map_or(true, |last| last != &input_text)
            {
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
                if let (Some(chat_id), Some(reply_to_id)) = (pane.chat_id, pane.reply_to_message) {
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
                        let _ = telegram
                            .reply_to_message(chat_id_copy, reply_to_id_copy, &input_text_copy)
                            .await;
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
                        let target_id = if self.panes.iter().any(|p| p.chat_id == Some(chat_id)) {
                            chat_id
                        } else {
                            normalized_id
                        };

                        // Fetch the updated message list in the background so the
                        // event loop never blocks on the network.
                        let telegram = self.telegram.clone();
                        let my_user_id = self.my_user_id;
                        let panes = matching_panes.clone();
                        self.spawn_op(async move {
                            match telegram.get_messages(target_id, 50).await {
                                Ok(raw) => OpResult::PaneReload {
                                    panes,
                                    msgs: message_data_from_raw(&raw, my_user_id),
                                    err: None,
                                },
                                Err(e) => OpResult::PaneReload {
                                    panes,
                                    msgs: Vec::new(),
                                    err: Some(e.to_string()),
                                },
                            }
                        });
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
                crate::telegram::TelegramUpdate::UserTyping { chat_id, user_name } => {
                    let normalized_id = crate::utils::normalize_chat_id(chat_id);
                    for pane in &mut self.panes {
                        if pane.chat_id == Some(chat_id) || pane.chat_id == Some(normalized_id) {
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

/// Truncate text to fit a width, appending an ellipsis
fn truncate_to_width(text: &str, max_width: usize) -> String {
    if text.width() <= max_width {
        return text.to_string();
    }
    if max_width <= 1 {
        return "…".to_string();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for c in text.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(1).max(1);
        if w + cw + 1 > max_width {
            break;
        }
        w += cw;
        out.push(c);
    }
    out.push('…');
    out
}

/// Split a long word into chunks that each fit within `max_width`
fn split_word_into_width_chunks(word: &str, max_width: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;
    for c in word.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(1).max(1);
        if current_w + cw > max_width && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
            current_w = 0;
        }
        current.push(c);
        current_w += cw;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Approximate number of wrapped lines for a text at a given width
/// (character-based wrapping, matching the cursor position logic)
fn wrapped_line_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    let mut lines = 1usize;
    let mut line_width = 0usize;
    for c in text.chars() {
        if c == '\n' {
            lines += 1;
            line_width = 0;
            continue;
        }
        let cw = UnicodeWidthChar::width(c).unwrap_or(1).max(1);
        if line_width + cw > width {
            lines += 1;
            line_width = cw;
        } else {
            line_width += cw;
        }
    }
    lines
}

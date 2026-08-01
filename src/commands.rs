use anyhow::Result;

use crate::app::{App, DeletePending};
use crate::widgets::FilterType;

pub struct Command {
    pub name: String,
    pub args: Vec<String>,
    pub _full_text: String,
}

impl Command {
    pub fn parse(text: &str) -> Option<Self> {
        if !text.starts_with('/') {
            return None;
        }

        let parts: Vec<&str> = text.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        let name = parts[0][1..].to_string();
        let args = parts[1..].iter().map(|s| s.to_string()).collect();

        Some(Command {
            name,
            args,
            _full_text: text.to_string(),
        })
    }
}

pub struct CommandHandler;

impl CommandHandler {
    pub async fn handle(app: &mut App, text: &str, pane_idx: usize) -> Result<bool> {
        let cmd = match Command::parse(text) {
            Some(c) => c,
            None => return Ok(false),
        };

        match cmd.name.as_str() {
            "reply" | "r" => {
                Self::handle_reply(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            "media" | "m" => {
                Self::handle_media(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            "edit" | "e" => {
                Self::handle_edit(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            "delete" | "del" | "d" => {
                Self::handle_delete(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            "alias" => {
                Self::handle_alias(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            "unalias" => {
                Self::handle_unalias(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            "filter" => {
                Self::handle_filter(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            "search" | "s" => {
                Self::handle_search(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            "new" => {
                Self::handle_new_chat(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            "newgroup" => {
                Self::handle_new_group(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            "add" => {
                Self::handle_add_member(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            "kick" | "remove" => {
                Self::handle_remove_member(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            "members" => {
                Self::handle_members(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            "forward" | "fwd" | "f" => {
                Self::handle_forward(app, &cmd, pane_idx).await?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Resolve a display number (#N) to the real Telegram message ID.
    /// Returns the message id if found and it is a real (sent) message.
    fn resolve_msg_id(app: &App, pane_idx: usize, msg_num: i32) -> Option<i32> {
        let pane = app.panes.get(pane_idx)?;
        let msg = pane.msg_data.get((msg_num - 1) as usize)?;
        if msg.msg_id <= 0 {
            return None;
        }
        Some(msg.msg_id)
    }

    async fn handle_reply(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.is_empty() {
            app.notify("Usage: /reply N [text]");
            return Ok(());
        }

        let msg_num: i32 = match cmd.args[0].trim_start_matches('#').parse() {
            Ok(n) => n,
            Err(_) => {
                app.notify("Usage: /reply N [text]");
                return Ok(());
            }
        };

        if let Some(pane) = app.panes.get_mut(pane_idx) {
            if cmd.args.len() > 1 {
                // Reply with inline text
                let text = cmd.args[1..].join(" ");
                if let Some(chat_id) = pane.chat_id {
                    match Self::resolve_msg_id(app, pane_idx, msg_num) {
                        Some(msg_id) => {
                            app.queue_reply_message(pane_idx, chat_id, msg_id, msg_num, text);
                        }
                        None => {
                            app.notify_error(&format!(
                                "Message #{} not found in current view",
                                msg_num
                            ));
                        }
                    }
                }
            } else {
                // Set reply mode with preview - find actual message ID from msg_data
                if let Some(msg_data) = pane.msg_data.get((msg_num - 1) as usize) {
                    let actual_msg_id = msg_data.msg_id;
                    if actual_msg_id <= 0 {
                        app.notify_error(&format!("Message #{} has not been sent yet", msg_num));
                        return Ok(());
                    }
                    pane.reply_to_message = Some(actual_msg_id);

                    // Get first line of message for preview (max 60 chars)
                    let first_line = msg_data.text.lines().next().unwrap_or(&msg_data.text);
                    let preview_text = if first_line.chars().count() > 60 {
                        let truncate_at = first_line
                            .char_indices()
                            .nth(60)
                            .map(|(i, _)| i)
                            .unwrap_or(first_line.len());
                        format!("{}...", &first_line[..truncate_at])
                    } else {
                        first_line.to_string()
                    };

                    pane.show_reply_preview(format!("Reply to #{}: {}", msg_num, preview_text));
                    app.notify(&format!(
                        "Replying to message #{}. Type your reply.",
                        msg_num
                    ));
                } else {
                    app.notify_error(&format!("Message #{} not found in current view", msg_num));
                }
            }
        }

        Ok(())
    }

    async fn handle_media(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.is_empty() {
            app.notify("Usage: /media N or /m N");
            return Ok(());
        }

        let msg_num: i32 = match cmd.args[0].trim_start_matches('#').parse() {
            Ok(n) => n,
            Err(_) => {
                app.notify("Usage: /media N");
                return Ok(());
            }
        };

        let (chat_id, msg_id) = if let Some(pane) = app.panes.get(pane_idx) {
            match pane.chat_id {
                Some(chat_id) => match Self::resolve_msg_id(app, pane_idx, msg_num) {
                    Some(msg_id) => (Some(chat_id), Some(msg_id)),
                    None => {
                        app.notify_error(&format!(
                            "Message #{} not found in current view",
                            msg_num
                        ));
                        return Ok(());
                    }
                },
                None => {
                    app.notify_error("No chat selected");
                    return Ok(());
                }
            }
        } else {
            return Ok(());
        };

        if let (Some(chat_id), Some(msg_id)) = (chat_id, msg_id) {
            app.notify(&format!("Downloading media from #{}...", msg_num));
            app.queue_media_download(pane_idx, chat_id, msg_id, msg_num);
        }

        Ok(())
    }

    async fn handle_edit(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.len() < 2 {
            app.notify("Usage: /edit N new_text");
            return Ok(());
        }

        let msg_num: i32 = match cmd.args[0].trim_start_matches('#').parse() {
            Ok(n) => n,
            Err(_) => {
                app.notify("Usage: /edit N new_text");
                return Ok(());
            }
        };

        let new_text = cmd.args[1..].join(" ");

        if let Some(pane) = app.panes.get(pane_idx) {
            if let Some(chat_id) = pane.chat_id {
                match Self::resolve_msg_id(app, pane_idx, msg_num) {
                    Some(msg_id) => {
                        app.queue_edit_message(pane_idx, chat_id, msg_id, msg_num, new_text);
                    }
                    None => {
                        app.notify_error(&format!(
                            "Message #{} not found in current view",
                            msg_num
                        ));
                    }
                }
            } else {
                app.notify_error("No chat selected");
            }
        }

        Ok(())
    }

    async fn handle_delete(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.is_empty() {
            app.notify("Usage: /delete N");
            return Ok(());
        }

        let msg_num: i32 = match cmd.args[0].trim_start_matches('#').parse() {
            Ok(n) => n,
            Err(_) => {
                app.notify("Usage: /delete N");
                return Ok(());
            }
        };

        if let Some(pane) = app.panes.get(pane_idx) {
            if let Some(chat_id) = pane.chat_id {
                match Self::resolve_msg_id(app, pane_idx, msg_num) {
                    Some(msg_id) => {
                        app.pending_delete = Some(DeletePending {
                            pane_idx,
                            chat_id,
                            chat_name: pane.chat_name.clone(),
                            msg_num,
                            actual_msg_id: msg_id,
                        });
                        app.notify_with_duration(&format!("Delete #{}? [y]es / [n]o", msg_num), 10);
                    }
                    None => {
                        app.notify_error(&format!(
                            "Message #{} not found in current view",
                            msg_num
                        ));
                    }
                }
            } else {
                app.notify_error("No chat selected");
            }
        }

        Ok(())
    }

    async fn handle_alias(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.len() < 2 {
            app.notify("Usage: /alias N name");
            return Ok(());
        }

        let msg_num: i32 = match cmd.args[0].trim_start_matches('#').parse() {
            Ok(n) => n,
            Err(_) => {
                app.notify("Usage: /alias N name");
                return Ok(());
            }
        };

        let alias = cmd.args[1..].join(" ");

        if let Some(pane) = app.panes.get(pane_idx) {
            if let Some(chat_id) = pane.chat_id {
                match Self::resolve_msg_id(app, pane_idx, msg_num) {
                    Some(msg_id) => {
                        app.queue_resolve_sender(pane_idx, chat_id, msg_id, Some(alias));
                    }
                    None => {
                        app.notify_error(&format!(
                            "Message #{} not found in current view",
                            msg_num
                        ));
                    }
                }
            } else {
                app.notify_error("No chat selected");
            }
        }

        Ok(())
    }

    async fn handle_unalias(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.is_empty() {
            app.notify("Usage: /unalias N");
            return Ok(());
        }

        let msg_num: i32 = match cmd.args[0].trim_start_matches('#').parse() {
            Ok(n) => n,
            Err(_) => {
                app.notify("Usage: /unalias N");
                return Ok(());
            }
        };

        if let Some(pane) = app.panes.get(pane_idx) {
            if let Some(chat_id) = pane.chat_id {
                match Self::resolve_msg_id(app, pane_idx, msg_num) {
                    Some(msg_id) => {
                        app.queue_resolve_sender(pane_idx, chat_id, msg_id, None);
                    }
                    None => {
                        app.notify_error(&format!(
                            "Message #{} not found in current view",
                            msg_num
                        ));
                    }
                }
            } else {
                app.notify_error("No chat selected");
            }
        }

        Ok(())
    }

    async fn handle_filter(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.is_empty() {
            if let Some(pane) = app.panes.get(pane_idx) {
                if pane.filter_type.is_some() {
                    let ft = match &pane.filter_type {
                        Some(FilterType::Sender) => "sender",
                        Some(FilterType::Media) => "media",
                        Some(FilterType::Link) => "link",
                        None => "",
                    };
                    let fv = pane.filter_value.as_deref().unwrap_or("");
                    app.notify(&format!("Current filter: {}={}", ft, fv));
                } else {
                    app.notify("Usage: /filter off | photo | video | audio | doc | link | <name>");
                }
            }
            return Ok(());
        }

        let filter_arg = cmd.args[0].to_lowercase();

        if filter_arg == "off" {
            if let Some(pane) = app.panes.get_mut(pane_idx) {
                pane.filter_type = None;
                pane.filter_value = None;
                pane.format_cache.clear();
            }
            app.notify("Filter disabled");
            return Ok(());
        }

        // Media type filters
        let media_types: &[(&str, &str)] = &[
            ("photo", "photo"),
            ("photos", "photo"),
            ("video", "video"),
            ("videos", "video"),
            ("audio", "audio"),
            ("voice", "voice"),
            ("doc", "document"),
            ("document", "document"),
            ("documents", "document"),
            ("file", "document"),
            ("files", "document"),
            ("link", "link"),
            ("links", "link"),
            ("url", "link"),
            ("sticker", "sticker"),
            ("stickers", "sticker"),
            ("gif", "gif"),
            ("gifs", "gif"),
        ];

        let notify_msg;
        if let Some((_, media_type)) = media_types.iter().find(|(k, _)| *k == filter_arg) {
            if let Some(pane) = app.panes.get_mut(pane_idx) {
                if *media_type == "link" {
                    pane.filter_type = Some(FilterType::Link);
                } else {
                    pane.filter_type = Some(FilterType::Media);
                }
                pane.filter_value = Some(media_type.to_string());
                pane.format_cache.clear();
            }
            notify_msg = format!("Filtering: {} only", media_type);
        } else {
            let filter_val = cmd.args.join(" ");
            notify_msg = format!("Filtering: messages from '{}'", filter_val);
            if let Some(pane) = app.panes.get_mut(pane_idx) {
                pane.filter_type = Some(FilterType::Sender);
                pane.filter_value = Some(filter_val);
                pane.format_cache.clear();
            }
        }
        app.notify(&notify_msg);

        Ok(())
    }

    async fn handle_search(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.is_empty() {
            app.notify("Usage: /search <query> or /s <query>  ·  /search off clears");
            return Ok(());
        }

        // /search off | /search clear exits search mode and restores the chat
        if cmd.args[0] == "off" || cmd.args[0] == "clear" {
            app.restore_search(pane_idx);
            return Ok(());
        }

        let query = cmd.args.join(" ");

        if app.panes.get(pane_idx).is_some_and(|p| p.chat_id.is_none()) {
            app.notify_error("Select a chat first");
            return Ok(());
        }

        let chat_id = app.panes.get(pane_idx).and_then(|p| p.chat_id);
        if let Some(chat_id) = chat_id {
            app.notify(&format!("Searching for '{}'...", query));
            app.queue_search(pane_idx, chat_id, query);
        }

        Ok(())
    }

    async fn handle_new_chat(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.is_empty() {
            app.notify("Usage: /new @username");
            return Ok(());
        }

        let username = cmd.args[0].clone();
        app.notify(&format!("Looking up {}...", username));
        app.queue_resolve_and_open(pane_idx, username);

        Ok(())
    }

    async fn handle_new_group(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.is_empty() {
            app.notify("Usage: /newgroup <name>");
            return Ok(());
        }

        let group_name = cmd.args.join(" ");
        app.notify(&format!("Creating group '{}'...", group_name));
        app.queue_create_group(pane_idx, group_name);

        Ok(())
    }

    async fn handle_add_member(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.is_empty() {
            app.notify("Usage: /add @username");
            return Ok(());
        }

        let username = cmd.args[0].clone();
        let chat_id = if let Some(pane) = app.panes.get(pane_idx) {
            match pane.chat_id {
                Some(id) => id,
                None => {
                    app.notify_error("Open a group chat first");
                    return Ok(());
                }
            }
        } else {
            return Ok(());
        };

        app.notify(&format!("Adding {}...", username));
        app.queue_add_remove(pane_idx, "add".to_string(), username, chat_id);

        Ok(())
    }

    async fn handle_remove_member(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.is_empty() {
            app.notify("Usage: /kick @username or /remove @username");
            return Ok(());
        }

        let username = cmd.args[0].clone();
        let chat_id = if let Some(pane) = app.panes.get(pane_idx) {
            match pane.chat_id {
                Some(id) => id,
                None => {
                    app.notify_error("Open a group chat first");
                    return Ok(());
                }
            }
        } else {
            return Ok(());
        };

        app.notify(&format!("Removing {}...", username));
        app.queue_add_remove(pane_idx, "remove".to_string(), username, chat_id);

        Ok(())
    }

    async fn handle_members(app: &mut App, _cmd: &Command, pane_idx: usize) -> Result<()> {
        let chat_id = if let Some(pane) = app.panes.get(pane_idx) {
            match pane.chat_id {
                Some(id) => id,
                None => {
                    app.notify_error("Open a group chat first");
                    return Ok(());
                }
            }
        } else {
            return Ok(());
        };

        app.notify("Loading members...");
        app.queue_members(pane_idx, chat_id);

        Ok(())
    }

    async fn handle_forward(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.len() < 2 {
            app.notify("Usage: /forward N @username or /fwd N @username");
            return Ok(());
        }

        let msg_num: i32 = match cmd.args[0].trim_start_matches('#').parse() {
            Ok(n) => n,
            Err(_) => {
                app.notify("Usage: /forward N @username");
                return Ok(());
            }
        };

        let target = cmd.args[1].clone();

        if let Some(pane) = app.panes.get(pane_idx) {
            let from_chat_id = match pane.chat_id {
                Some(id) => id,
                None => {
                    app.notify_error("No chat selected");
                    return Ok(());
                }
            };
            let message_id = match Self::resolve_msg_id(app, pane_idx, msg_num) {
                Some(id) => id,
                None => {
                    app.notify_error(&format!("Message #{} not found in current view", msg_num));
                    return Ok(());
                }
            };
            app.notify(&format!("Forwarding #{} to {}...", msg_num, target));
            app.queue_forward(pane_idx, msg_num, target, from_chat_id, message_id);
        }

        Ok(())
    }
}

use anyhow::Result;

use crate::app::App;
use crate::widgets::FilterType;

pub struct Command {
    pub name: String,
    pub args: Vec<String>,
    pub full_text: String,
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
            full_text: text.to_string(),
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
                    match app
                        .telegram
                        .reply_to_message(chat_id, msg_num, &text)
                        .await
                    {
                        Ok(_) => pane.add_message(format!("✓ Replied to #{}", msg_num)),
                        Err(e) => pane.add_message(format!("✗ Reply failed: {}", e)),
                    }
                }
            } else {
                // Set reply mode with preview - find actual message ID from msg_data
                if let Some(msg_data) = pane.msg_data.get((msg_num - 1) as usize) {
                    let actual_msg_id = msg_data.msg_id;
                    pane.reply_to_message = Some(actual_msg_id);
                    
                    // Get first line of message for preview (max 60 chars)
                    let first_line = msg_data.text.lines().next().unwrap_or(&msg_data.text);
                    let preview_text = if first_line.chars().count() > 60 {
                        let truncate_at = first_line.char_indices().nth(60).map(|(i, _)| i).unwrap_or(first_line.len());
                        format!("{}...", &first_line[..truncate_at])
                    } else {
                        first_line.to_string()
                    };
                    
                    pane.show_reply_preview(format!("Reply to #{}: {}", msg_num, preview_text));
                    app.notify(&format!("Replying to message #{}. Type your reply.", msg_num));
                } else {
                    pane.add_message(format!("✗ Message #{} not found", msg_num));
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

        let chat_id = app.panes.get(pane_idx).and_then(|p| p.chat_id);
        if let Some(chat_id) = chat_id {
            app.notify(&format!("Downloading media from #{}...", msg_num));
            let downloads_dir = std::env::temp_dir();

            match app
                .telegram
                .download_media(chat_id, msg_num, &downloads_dir)
                .await
            {
                Ok(path) => {
                    if let Some(pane) = app.panes.get_mut(pane_idx) {
                        pane.add_message(format!("✓ Downloaded to {}", path));
                    }
                    #[cfg(target_os = "macos")]
                    {
                        let _ = std::process::Command::new("open").arg(&path).spawn();
                    }
                    #[cfg(target_os = "linux")]
                    {
                        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                    }
                    app.notify(&format!(
                        "Opened: {}",
                        std::path::Path::new(&path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    ));
                }
                Err(e) => {
                    if let Some(pane) = app.panes.get_mut(pane_idx) {
                        pane.add_message(format!("✗ Download failed: {}", e));
                    }
                    app.notify(&format!("Download failed: {}", e));
                }
            }
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

        if let Some(pane) = app.panes.get_mut(pane_idx) {
            if let Some(chat_id) = pane.chat_id {
                match app
                    .telegram
                    .edit_message(chat_id, msg_num, &new_text)
                    .await
                {
                    Ok(_) => {
                        pane.add_message(format!("✓ Edited message #{}", msg_num));
                        app.notify("Message edited");
                    }
                    Err(e) => {
                        pane.add_message(format!("✗ Edit failed: {}", e));
                        app.notify(&format!("Edit failed: {}", e));
                    }
                }
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

        if let Some(pane) = app.panes.get_mut(pane_idx) {
            if let Some(chat_id) = pane.chat_id {
                match app.telegram.delete_message(chat_id, msg_num).await {
                    Ok(_) => {
                        pane.add_message(format!("✓ Deleted message #{}", msg_num));
                        app.notify("Message deleted");
                    }
                    Err(e) => {
                        pane.add_message(format!("✗ Delete failed: {}", e));
                        app.notify(&format!("Delete failed: {}", e));
                    }
                }
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

        if let Some(pane) = app.panes.get_mut(pane_idx) {
            if let Some(chat_id) = pane.chat_id {
                let sender_id = app
                    .telegram
                    .get_message_sender(chat_id, msg_num)
                    .await?;
                if let Some(sender_id) = sender_id {
                    app.aliases.insert(sender_id, alias.clone());
                    app.aliases.save(&app.config)?;
                    pane.add_message(format!("✓ Alias set: {}", alias));
                    app.notify(&format!("Alias set: {}", alias));
                } else {
                    pane.add_message("✗ Could not find message sender".to_string());
                    app.notify("Could not find message sender");
                }
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

        if let Some(pane) = app.panes.get_mut(pane_idx) {
            if let Some(chat_id) = pane.chat_id {
                let sender_id = app
                    .telegram
                    .get_message_sender(chat_id, msg_num)
                    .await?;
                if let Some(sender_id) = sender_id {
                    if app.aliases.remove(&sender_id).is_some() {
                        app.aliases.save(&app.config)?;
                        pane.add_message("✓ Alias removed".to_string());
                        app.notify("Alias removed");
                    } else {
                        pane.add_message("✗ No alias found".to_string());
                        app.notify("No alias set for this user");
                    }
                } else {
                    pane.add_message("✗ Could not find message sender".to_string());
                    app.notify("Could not find message sender");
                }
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
            app.notify("Usage: /search <query> or /s <query>");
            return Ok(());
        }

        let query = cmd.args.join(" ");

        if let Some(pane) = app.panes.get(pane_idx) {
            if pane.chat_id.is_none() {
                app.notify("Select a chat first");
                return Ok(());
            }
        }

        let chat_id = app.panes.get(pane_idx).and_then(|p| p.chat_id);
        if let Some(chat_id) = chat_id {
            app.notify(&format!("Searching for '{}'...", query));

            match app.telegram.search_messages(chat_id, &query, 100).await {
                Ok(results) => {
                    let count = results.len();
                    if count == 0 {
                        app.notify("No results found");
                    } else {
                        // Convert to MessageData for proper formatting support
                        let msg_data: Vec<crate::widgets::MessageData> = results
                            .iter()
                            .map(|(msg_id, sender_id, sender_name, text, reply_to_id, reactions)| {
                                let reply_to_msg_id = *reply_to_id;
                                
                                crate::widgets::MessageData {
                                    msg_id: *msg_id,
                                    sender_id: *sender_id,
                                    sender_name: sender_name.clone(),
                                    text: text.clone(),
                                    is_outgoing: *sender_id == app.my_user_id,
                                    timestamp: chrono::Utc::now().timestamp(),
                                    media_type: None,
                                    media_label: None,
                                    reactions: reactions.clone(),
                                    reply_to_msg_id,
                                    reply_sender: None,
                                    reply_text: None,
                                }
                            })
                            .collect();

                        if let Some(pane) = app.panes.get_mut(pane_idx) {
                            pane.msg_data = msg_data;
                            // Don't clear messages - they may contain status messages
                            pane.chat_name = format!(
                                "{} | Search: '{}' ({} results)",
                                pane.chat_name.split(" | Search:").next().unwrap_or(&pane.chat_name),
                                query,
                                count
                            );
                            pane.scroll_offset = 0;
                        }
                        app.notify(&format!("Found {} results", count));
                    }
                }
                Err(e) => {
                    app.notify(&format!("Search failed: {}", e));
                }
            }
        }

        Ok(())
    }

    async fn handle_new_chat(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.is_empty() {
            app.notify("Usage: /new @username");
            return Ok(());
        }

        let username = &cmd.args[0];

        if let Some(pane) = app.panes.get_mut(pane_idx) {
            pane.add_message(format!("Starting chat with {}...", username));
            app.notify(&format!("Looking up {}...", username));
            // Full implementation would resolve username via Telegram API and open the chat
        }

        Ok(())
    }

    async fn handle_new_group(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.is_empty() {
            app.notify("Usage: /newgroup <name>");
            return Ok(());
        }

        let group_name = cmd.args.join(" ");

        if let Some(pane) = app.panes.get_mut(pane_idx) {
            pane.add_message(format!("Creating group: {}...", group_name));
            app.notify(&format!("Creating group '{}'...", group_name));
            // Full implementation would call Telegram's CreateChatRequest
        }

        Ok(())
    }

    async fn handle_add_member(app: &mut App, cmd: &Command, pane_idx: usize) -> Result<()> {
        if cmd.args.is_empty() {
            app.notify("Usage: /add @username");
            return Ok(());
        }

        let username = &cmd.args[0];

        if let Some(pane) = app.panes.get(pane_idx) {
            if pane.chat_id.is_none() {
                app.notify("Open a group chat first");
                return Ok(());
            }
        }

        if let Some(pane) = app.panes.get_mut(pane_idx) {
            pane.add_message(format!("Adding {} to group...", username));
            app.notify(&format!("Adding {}...", username));
            // Full implementation would call InviteToChannelRequest or AddChatUserRequest
        }

        Ok(())
    }

    async fn handle_remove_member(
        app: &mut App,
        cmd: &Command,
        pane_idx: usize,
    ) -> Result<()> {
        if cmd.args.is_empty() {
            app.notify("Usage: /kick @username or /remove @username");
            return Ok(());
        }

        let username = &cmd.args[0];

        if let Some(pane) = app.panes.get(pane_idx) {
            if pane.chat_id.is_none() {
                app.notify("Open a group chat first");
                return Ok(());
            }
        }

        if let Some(pane) = app.panes.get_mut(pane_idx) {
            pane.add_message(format!("Removing {} from group...", username));
            app.notify(&format!("Removing {}...", username));
            // Full implementation would call EditBannedRequest or kick_participant
        }

        Ok(())
    }

    async fn handle_members(app: &mut App, _cmd: &Command, pane_idx: usize) -> Result<()> {
        if let Some(pane) = app.panes.get(pane_idx) {
            if pane.chat_id.is_none() {
                app.notify("Open a group chat first");
                return Ok(());
            }
        }

        if let Some(pane) = app.panes.get_mut(pane_idx) {
            pane.add_message("Fetching group members...".to_string());
            app.notify("Loading members...");
            // Full implementation would call get_participants
        }

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

        let target = &cmd.args[1];

        if let Some(pane) = app.panes.get_mut(pane_idx) {
            if let Some(_from_chat_id) = pane.chat_id {
                pane.add_message(format!(
                    "Forwarding message #{} to {}...",
                    msg_num, target
                ));
                app.notify(&format!("Forwarding #{} to {}...", msg_num, target));
                // Full implementation would resolve target username, get to_chat_id,
                // then call app.telegram.forward_message(from_chat_id, msg_num, to_chat_id)
            }
        }

        Ok(())
    }
}

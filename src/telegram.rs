use anyhow::Result;
use grammers_client::{Client, Config as ClientConfig, InitParams, SignInError, Update};
use grammers_session::Session;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::app::ChatInfo;
use crate::config::Config;

/// Updates received from Telegram
pub enum TelegramUpdate {
    NewMessage {
        chat_id: i64,
        sender_name: String,
        text: String,
        is_outgoing: bool,
    },
    UserTyping {
        chat_id: i64,
        user_name: String,
    },
}

#[derive(Clone)]
pub struct TelegramClient {
    client: Arc<Mutex<Client>>,
    update_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pending_updates: Arc<Mutex<Vec<TelegramUpdate>>>,
}

impl TelegramClient {
    pub async fn new(config: &Config) -> Result<Self> {
        // Ensure config directory exists before trying to load/save session
        std::fs::create_dir_all(&config.config_dir)?;

        let session_path = config.session_path();
        let session = if session_path.exists() {
            Session::load_file(&session_path)?
        } else {
            Session::new()
        };

        let client_config = ClientConfig {
            session,
            api_id: config.api_id,
            api_hash: config.api_hash.clone(),
            params: InitParams {
                ..Default::default()
            },
        };

        let client = Client::connect(client_config).await?;

        // Check if we're authorized
        if !client.is_authorized().await? {
            Self::sign_in(&client).await?;
        }

        // Always save session after connecting
        let session_data = client.session().save();
        std::fs::write(&session_path, &session_data)?;

        Ok(Self {
            update_handle: Arc::new(Mutex::new(None)),
            pending_updates: Arc::new(Mutex::new(Vec::new())),
            client: Arc::new(Mutex::new(client)),
        })
    }

    async fn sign_in(client: &Client) -> Result<()> {
        use std::io::{self, Write};

        print!("Enter your phone number (international format): ");
        io::stdout().flush()?;
        let mut phone = String::new();
        io::stdin().read_line(&mut phone)?;
        let phone = phone.trim();

        let token = client.request_login_code(phone).await?;

        print!("Enter the code you received: ");
        io::stdout().flush()?;
        let mut code = String::new();
        io::stdin().read_line(&mut code)?;
        let code = code.trim();

        match client.sign_in(&token, code).await {
            Ok(_) => {}
            Err(SignInError::PasswordRequired(password_token)) => {
                print!("Enter your 2FA password: ");
                io::stdout().flush()?;
                let mut password = String::new();
                io::stdin().read_line(&mut password)?;
                let password = password.trim();

                client
                    .check_password(password_token, password)
                    .await?;
            }
            Err(e) => return Err(e.into()),
        }

        Ok(())
    }

    pub async fn get_me(&self) -> Result<i64> {
        let client = self.client.lock().await;
        let user = client.get_me().await?;
        Ok(user.id())
    }

    pub async fn get_dialogs(&self) -> Result<Vec<ChatInfo>> {
        let client = self.client.lock().await;
        let mut chats = Vec::new();

        let mut dialogs = client.iter_dialogs();
        while let Some(dialog) = dialogs.next().await? {
            let chat = dialog.chat();

            let chat_type = match chat {
                grammers_client::types::Chat::User(_) => (false, false),
                grammers_client::types::Chat::Group(_) => (false, true),
                grammers_client::types::Chat::Channel(_) => (true, false),
            };

            // Extract username
            let username = match chat {
                grammers_client::types::Chat::User(u) => {
                    u.username().map(|u| format!("@{}", u))
                }
                grammers_client::types::Chat::Channel(c) => {
                    c.username().map(|u| format!("@{}", u))
                }
                _ => None,
            };

            // Get chat name with fallback for empty names
            let chat_name = chat.name().to_string();
            let display_name = if chat_name.trim().is_empty() {
                username.as_ref()
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| format!("Chat {}", chat.id()))
            } else {
                chat_name
            };

            chats.push(ChatInfo {
                id: chat.id(),
                name: display_name,
                username,
                unread: match &dialog.raw {
                    grammers_client::grammers_tl_types::enums::Dialog::Dialog(d) => d.unread_count as u32,
                    _ => 0,
                },
                is_channel: chat_type.0,
                is_group: chat_type.1,
            });
        }

        Ok(chats)
    }

    pub async fn get_messages(
        &self,
        chat_id: i64,
        limit: usize,
    ) -> Result<Vec<(i32, i64, String, String, Option<i32>, Option<String>, std::collections::HashMap<String, u32>)>> {
        let client = self.client.lock().await;

        let chat = match self.find_chat_inner(&client, chat_id).await? {
            Some(c) => c,
            None => return Ok(Vec::new()),
        };

        let mut messages = Vec::new();
        let mut iter = client.iter_messages(&chat);

        let mut count = 0;
        while let Some(message) = iter.next().await? {
            if count >= limit {
                break;
            }

            let text = message.text();
            let (sender_id, sender_name) = if let Some(sender) = message.sender() {
                (sender.id(), sender.name().to_string())
            } else {
                (0, "Unknown".to_string())
            };

            // Check if this is a reply
            let reply_to_id = message.reply_to_message_id();

            // Detect media type
            let media_type = if let Some(media) = message.media() {
                use grammers_client::types::Media;
                Some(match media {
                    Media::Photo(_) => "photo".to_string(),
                    Media::Document(doc) => {
                        if let Some(mime) = doc.mime_type() {
                            if mime.starts_with("video/") {
                                "video".to_string()
                            } else if mime.starts_with("audio/") {
                                "audio".to_string()
                            } else {
                                "document".to_string()
                            }
                        } else {
                            "document".to_string()
                        }
                    }
                    Media::Contact(_) => "contact".to_string(),
                    Media::Dice(_) => "dice".to_string(),
                    Media::Poll(_) => "poll".to_string(),
                    Media::Venue(_) => "location".to_string(),
                    Media::Sticker(_) => "sticker".to_string(),
                    _ => "media".to_string(),
                })
            } else {
                None
            };

            // Get reactions from message
            let mut reactions = std::collections::HashMap::new();
            if let Some(raw_reactions) = &message.raw.reactions {
                use grammers_tl_types::enums::MessageReactions;
                let MessageReactions::Reactions(reactions_data) = raw_reactions;
                for reaction_count in &reactions_data.results {
                    use grammers_tl_types::enums::ReactionCount;
                    let ReactionCount::Count(count_data) = reaction_count;
                    let emoji = match &count_data.reaction {
                        grammers_tl_types::enums::Reaction::Emoji(emoji_data) => {
                            emoji_data.emoticon.clone()
                        }
                        grammers_tl_types::enums::Reaction::CustomEmoji(custom_emoji) => {
                            format!("[emoji:{:?}]", custom_emoji)
                        }
                        _ => continue,
                    };
                    *reactions.entry(emoji).or_insert(0) += count_data.count as u32;
                }
            }

            // Include messages with text or media
            if !text.is_empty() || media_type.is_some() {
                messages.push((
                    message.id(),
                    sender_id,
                    sender_name,
                    text.to_string(),
                    reply_to_id,
                    media_type,
                    reactions,
                ));
            }

            count += 1;
        }

        messages.reverse();
        Ok(messages)
    }

    pub async fn send_message(&self, chat_id: i64, text: &str) -> Result<()> {
        let client = self.client.lock().await;
        let chat = self.find_chat_inner(&client, chat_id).await?;

        if let Some(chat) = chat {
            client.send_message(&chat, text).await?;
        }

        Ok(())
    }

    pub async fn reply_to_message(
        &self,
        chat_id: i64,
        message_id: i32,
        text: &str,
    ) -> Result<()> {
        let client = self.client.lock().await;
        let chat = self.find_chat_inner(&client, chat_id).await?;

        if let Some(chat) = chat {
            use grammers_client::InputMessage;
            let input = InputMessage::text(text).reply_to(Some(message_id));
            client.send_message(&chat, input).await?;
        }

        Ok(())
    }

    pub async fn edit_message(
        &self,
        chat_id: i64,
        message_id: i32,
        new_text: &str,
    ) -> Result<()> {
        let client = self.client.lock().await;
        let chat = self.find_chat_inner(&client, chat_id).await?;

        if let Some(chat) = chat {
            // Find the message to edit
            let mut iter = client.iter_messages(&chat);
            while let Some(message) = iter.next().await? {
                if message.id() == message_id {
                    use grammers_client::InputMessage;
                    let input = InputMessage::text(new_text);
                    client.edit_message(&chat, message_id, input).await?;
                    break;
                }
            }
        }

        Ok(())
    }

    pub async fn delete_message(&self, chat_id: i64, message_id: i32) -> Result<()> {
        let client = self.client.lock().await;
        let chat = self.find_chat_inner(&client, chat_id).await?;

        if let Some(chat) = chat {
            client.delete_messages(&chat, &[message_id]).await?;
        }

        Ok(())
    }

    pub async fn forward_message(
        &self,
        from_chat_id: i64,
        message_id: i32,
        to_chat_id: i64,
    ) -> Result<()> {
        let client = self.client.lock().await;
        let from_chat = self.find_chat_inner(&client, from_chat_id).await?;
        let to_chat = self.find_chat_inner(&client, to_chat_id).await?;

        if let (Some(from), Some(to)) = (from_chat, to_chat) {
            client
                .forward_messages(&to, &[message_id], &from)
                .await?;
        }

        Ok(())
    }

    pub async fn download_media(
        &self,
        chat_id: i64,
        message_num: i32,  // Message number in display (#1, #2, etc)
        path: &std::path::Path,
    ) -> Result<String> {
        let client = self.client.lock().await;
        let chat = self.find_chat_inner(&client, chat_id).await?;

        if let Some(chat) = chat {
            // Get all messages to find the one at message_num position
            let mut messages_vec = Vec::new();
            let mut iter = client.iter_messages(&chat);
            while let Some(message) = iter.next().await? {
                messages_vec.push(message);
                if messages_vec.len() >= 100 {
                    break; // Limit to last 100 messages
                }
            }
            messages_vec.reverse();

            // Get message at position (message_num - 1)
            if let Some(message) = messages_vec.get((message_num - 1) as usize) {
                if let Some(media) = message.media() {
                    // Determine file extension based on media type
                    use grammers_client::types::Media;
                    let ext = match &media {
                        Media::Photo(_) => "jpg",
                        Media::Document(doc) => {
                            if let Some(mime) = doc.mime_type() {
                                if mime.starts_with("video/") {
                                    "mp4"
                                } else if mime.starts_with("audio/") {
                                    "mp3"
                                } else {
                                    "dat"
                                }
                            } else {
                                "dat"
                            }
                        }
                        _ => "dat",
                    };

                    let download_path = path.join(format!("telegram_msg_{}_{}.{}", chat_id, message_num, ext));
                    
                    // Download media
                    use grammers_client::types::Downloadable;
                    let mut download = client.iter_download(&Downloadable::Media(media));
                    let mut buf = Vec::new();
                    while let Some(chunk) = download.next().await? {
                        buf.extend_from_slice(&chunk);
                    }
                    std::fs::write(&download_path, &buf)?;
                    return Ok(download_path.to_string_lossy().to_string());
                } else {
                    anyhow::bail!("Message #{} has no media", message_num);
                }
            } else {
                anyhow::bail!("Message #{} not found (only last 100 messages available)", message_num);
            }
        }

        anyhow::bail!("Chat not found")
    }

    pub async fn search_messages(
        &self,
        chat_id: i64,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(i32, i64, String, String, Option<i32>, std::collections::HashMap<String, u32>)>> {
        let client = self.client.lock().await;
        let chat = self.find_chat_inner(&client, chat_id).await?;

        if let Some(chat) = chat {
            let mut messages = Vec::new();
            let mut iter = client.search_messages(&chat).query(query);

            let mut count = 0;
            while let Some(message) = iter.next().await? {
                if count >= limit {
                    break;
                }

                let text = message.text();
                let (sender_id, sender_name) = if let Some(sender) = message.sender() {
                    (sender.id(), sender.name().to_string())
                } else {
                    (0, "Unknown".to_string())
                };

                let reply_to_id = message.reply_to_message_id();

                // Get reactions from message
                let mut reactions = std::collections::HashMap::new();
                if let Some(raw_reactions) = &message.raw.reactions {
                    use grammers_tl_types::enums::MessageReactions;
                    let MessageReactions::Reactions(reactions_data) = raw_reactions;
                    for reaction_count in &reactions_data.results {
                        use grammers_tl_types::enums::ReactionCount;
                        let ReactionCount::Count(count_data) = reaction_count;
                        let emoji = match &count_data.reaction {
                            grammers_tl_types::enums::Reaction::Emoji(emoji_data) => {
                                emoji_data.emoticon.clone()
                            }
                            grammers_tl_types::enums::Reaction::CustomEmoji(custom_emoji) => {
                                format!("[emoji:{:?}]", custom_emoji)
                            }
                            _ => continue,
                        };
                        *reactions.entry(emoji).or_insert(0) += count_data.count as u32;
                    }
                }

                if !text.is_empty() {
                    messages.push((message.id(), sender_id, sender_name, text.to_string(), reply_to_id, reactions));
                }
                count += 1;
            }

            messages.reverse();
            return Ok(messages);
        }

        Ok(Vec::new())
    }

    pub async fn get_message_sender(
        &self,
        chat_id: i64,
        message_id: i32,
    ) -> Result<Option<i64>> {
        let client = self.client.lock().await;
        let chat = self.find_chat_inner(&client, chat_id).await?;

        if let Some(chat) = chat {
            let mut iter = client.iter_messages(&chat);
            while let Some(message) = iter.next().await? {
                if message.id() == message_id {
                    if let Some(sender) = message.sender() {
                        return Ok(Some(sender.id()));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Find a chat by iterating dialogs (internal helper that takes a locked client)
    async fn find_chat_inner(
        &self,
        client: &Client,
        chat_id: i64,
    ) -> Result<Option<grammers_client::types::Chat>> {
        let mut dialogs = client.iter_dialogs();

        while let Some(dialog) = dialogs.next().await? {
            if dialog.chat().id() == chat_id {
                return Ok(Some(dialog.chat().clone()));
            }
        }

        Ok(None)
    }

    /// Find a chat by ID (public API, acquires lock)
    pub async fn find_chat(
        &self,
        chat_id: i64,
    ) -> Result<Option<grammers_client::types::Chat>> {
        let client = self.client.lock().await;
        self.find_chat_inner(&client, chat_id).await
    }

    /// Poll for updates and return them. Non-blocking.
    pub async fn poll_updates(&self) -> Result<Vec<TelegramUpdate>> {
        // Start background listener if not already running
        let mut handle = self.update_handle.lock().await;

        if handle.is_none() {
            let client = Arc::clone(&self.client);
            let updates = Arc::clone(&self.pending_updates);

            let task = tokio::spawn(async move {
                loop {
                    let client_lock = client.lock().await;

                    // Get updates - grammers-client 0.7 only exposes NewMessage via next_update()
                    // In Python (telethon/pyrogram), typing updates come through iter_updates()
                    // but grammers-client doesn't have that method
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(100),
                        client_lock.next_update(),
                    )
                    .await
                    {
                        Ok(Ok(update)) => {
                            match update {
                                Update::NewMessage(msg) if !msg.outgoing() => {
                                    let chat_id = msg.chat().id();
                                    let sender_name = msg
                                        .sender()
                                        .map(|s| s.name().to_string())
                                        .unwrap_or_else(|| "Unknown".to_string());
                                    let text = msg.text().to_string();

                                    drop(client_lock);
                                    let mut pending = updates.lock().await;
                                    pending.push(TelegramUpdate::NewMessage {
                                        chat_id,
                                        sender_name,
                                        text,
                                        is_outgoing: false,
                                    });
                                }
                                Update::NewMessage(msg) if msg.outgoing() => {
                                    let chat_id = msg.chat().id();
                                    let text = msg.text().to_string();

                                    drop(client_lock);
                                    let mut pending = updates.lock().await;
                                    pending.push(TelegramUpdate::NewMessage {
                                        chat_id,
                                        sender_name: "You".to_string(),
                                        text,
                                        is_outgoing: true,
                                    });
                                }
                                _ => {
                                    drop(client_lock);
                                }
                            }
                        }
                        Ok(Err(_e)) => {
                            drop(client_lock);
                            break;
                        }
                        Err(_) => {
                            drop(client_lock);
                            // Timeout, continue
                        }
                    }

                    // Note: grammers-client 0.7 doesn't expose UserTyping updates
                    // In Python (telethon/pyrogram), typing updates come through iter_updates()
                    // but grammers-client doesn't have that method - only next_update()
                    // The Update enum only has NewMessage variant
                    // Typing indicators UI is ready (widgets.rs, app.rs) but can't receive updates yet
                    // 
                    // To fix this, we would need to:
                    // 1. Check if there's a newer version of grammers-client that supports typing
                    // 2. Use raw TL types directly (if Client exposes them somehow)
                    // 3. Wait for library support
                    
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            });

            *handle = Some(task);
        }
        drop(handle);

        // Drain pending updates
        let mut pending = self.pending_updates.lock().await;
        let updates = std::mem::take(&mut *pending);
        Ok(updates)
    }

    pub async fn save_session(&self, path: &std::path::Path) -> Result<()> {
        let client = self.client.lock().await;
        client.session().save_to_file(path)?;
        Ok(())
    }
}

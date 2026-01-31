use chrono::{DateTime, Local};

pub fn format_message_time(timestamp: i64) -> String {
    use chrono::Utc;

    let datetime_utc: DateTime<chrono::Utc> =
        DateTime::from_timestamp(timestamp, 0).unwrap_or_else(|| Utc::now());
    let datetime: DateTime<Local> = datetime_utc.into();

    let now = Local::now();
    let is_today = datetime.date_naive() == now.date_naive();

    if is_today {
        datetime.format("%H:%M").to_string()
    } else {
        datetime.format("%Y-%m-%d %H:%M").to_string()
    }
}

pub fn log_message(message: &str, level: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;

    let log_file = "telegram_client.log";
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
    let log_line = format!("[{}] {}: {}\n", timestamp, level, message);

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)
    {
        let _ = file.write_all(log_line.as_bytes());
    }
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::utils::log_message(&format!($($arg)*), "ERROR")
    };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            $crate::utils::log_message(&format!($($arg)*), "DEBUG")
        }
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::utils::log_message(&format!($($arg)*), "INFO")
    };
}

pub fn sanitize_chat_name(name: &str) -> String {
    name.replace('[', "\\[")
        .replace(']', "\\]")
        .trim()
        .to_string()
}

pub fn get_user_display_name(
    first_name: Option<&str>,
    last_name: Option<&str>,
    username: Option<&str>,
    user_id: i64,
) -> String {
    if let Some(first) = first_name {
        let mut name = first.to_string();
        if let Some(last) = last_name {
            name.push(' ');
            name.push_str(last);
        }
        if !name.is_empty() {
            return name;
        }
    }

    if let Some(user) = username {
        return format!("@{}", user);
    }

    format!("User {}", user_id)
}

/// Send a desktop notification (macOS and Linux)
pub fn send_desktop_notification(title: &str, message: &str) {
    use std::process::Command;

    #[cfg(target_os = "macos")]
    {
        let safe_title = title.replace('"', "\\\"");
        let safe_msg = message.replace('"', "\\\"");
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            safe_msg, safe_title
        );
        let _ = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("notify-send")
            .arg("--app-name=Telegram Client")
            .arg("--urgency=normal")
            .arg("--expire-time=5000")
            .arg(title)
            .arg(message)
            .output();
    }
}

/// Normalize Telegram chat ID (channels use -100XXX format)
pub fn normalize_chat_id(raw_id: i64) -> i64 {
    if raw_id < 0 {
        let abs_id = raw_id.unsigned_abs();
        let str_id = abs_id.to_string();
        if str_id.starts_with("100") && str_id.len() > 3 {
            if let Ok(normalized) = str_id[3..].parse::<i64>() {
                return normalized;
            }
        }
        return abs_id as i64;
    }
    raw_id
}

/// Available commands for autocomplete
pub const COMMANDS: &[&str] = &[
    "/reply ",
    "/media ",
    "/m ",
    "/edit ",
    "/e ",
    "/delete ",
    "/del ",
    "/d ",
    "/alias ",
    "/unalias ",
    "/filter ",
    "/search ",
    "/s ",
    "/new ",
    "/newgroup ",
    "/add ",
    "/kick ",
    "/remove ",
    "/members",
    "/forward ",
    "/fwd ",
    "/f ",
];

/// Try to autocomplete a command prefix. Returns (completed_text, options_hint)
pub fn try_autocomplete(text: &str) -> (Option<String>, Option<String>) {
    if !text.starts_with('/') {
        return (None, None);
    }

    let matches: Vec<&&str> = COMMANDS.iter().filter(|cmd| cmd.starts_with(text)).collect();

    if matches.len() == 1 {
        return (Some(matches[0].to_string()), None);
    }

    if matches.len() > 1 {
        // Find common prefix
        let mut common = matches[0].to_string();
        for m in &matches[1..] {
            while !m.starts_with(&common) {
                common.pop();
            }
        }
        if common.len() > text.len() {
            return (Some(common), None);
        }
        let options = matches
            .iter()
            .map(|m| m.trim())
            .collect::<Vec<_>>()
            .join(", ");
        return (None, Some(format!("Options: {}", options)));
    }

    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_chat_name() {
        assert_eq!(sanitize_chat_name("Normal Name"), "Normal Name");
        assert_eq!(sanitize_chat_name("[Test]"), "\\[Test\\]");
        assert_eq!(sanitize_chat_name("  Spaces  "), "Spaces");
    }

    #[test]
    fn test_get_user_display_name() {
        assert_eq!(
            get_user_display_name(Some("John"), Some("Doe"), None, 123),
            "John Doe"
        );
        assert_eq!(
            get_user_display_name(None, None, Some("johndoe"), 123),
            "@johndoe"
        );
        assert_eq!(
            get_user_display_name(None, None, None, 123),
            "User 123"
        );
    }

    #[test]
    fn test_normalize_chat_id() {
        // Channel IDs: -100XXXXXXXXXX -> XXXXXXXXXX
        assert_eq!(normalize_chat_id(-1001234567890), 1234567890);
        // Regular group IDs: -XXXXXXXXXX -> XXXXXXXXXX
        assert_eq!(normalize_chat_id(-1234567), 1234567);
        // Positive IDs unchanged
        assert_eq!(normalize_chat_id(1234567), 1234567);
    }

    #[test]
    fn test_autocomplete() {
        let (result, _) = try_autocomplete("/rep");
        assert_eq!(result, Some("/reply ".to_string()));

        let (result, hint) = try_autocomplete("/f");
        // Multiple matches: /filter, /forward, /fwd, /f
        assert!(result.is_some() || hint.is_some());
    }
}

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub api_id: i32,
    pub api_hash: String,
    pub phone_number: Option<String>,
    
    #[serde(default)]
    pub settings: Settings,
    
    #[serde(skip)]
    pub config_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_true")]
    pub show_reactions: bool,
    
    #[serde(default = "default_true")]
    pub show_notifications: bool,
    
    #[serde(default)]
    pub compact_mode: bool,
    
    #[serde(default = "default_true")]
    pub show_emojis: bool,
    
    #[serde(default)]
    pub show_line_numbers: bool,
    
    #[serde(default = "default_true")]
    pub show_timestamps: bool,
    
    #[serde(default = "default_true")]
    pub show_user_colors: bool,
    
    #[serde(default = "default_true")]
    pub show_borders: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_reactions: true,
            show_notifications: true,
            compact_mode: false,
            show_emojis: true,
            show_line_numbers: false,
            show_timestamps: true,
            show_user_colors: true,
            show_borders: true,
        }
    }
}

fn default_true() -> bool {
    true
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_dir = Self::get_config_dir();
        let config_path = config_dir.join("telegram_config.json");

        if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            let mut config: Config = serde_json::from_str(&content)?;
            config.config_dir = config_dir;
            Ok(config)
        } else {
            // Create new config
            let config = Self::create_new(config_dir)?;
            Ok(config)
        }
    }

    pub fn save(&self) -> Result<()> {
        let config_path = self.config_dir.join("telegram_config.json");
        let content = serde_json::to_string_pretty(&self)?;
        fs::write(config_path, content)?;
        Ok(())
    }

    fn create_new(config_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&config_dir)?;

        println!("=== Telegram Client Setup ===");
        println!("Get your API credentials from https://my.telegram.org");
        
        print!("Enter API ID: ");
        use std::io::{self, Write};
        io::stdout().flush()?;
        let mut api_id_str = String::new();
        io::stdin().read_line(&mut api_id_str)?;
        let api_id: i32 = api_id_str.trim().parse()?;

        print!("Enter API Hash: ");
        io::stdout().flush()?;
        let mut api_hash = String::new();
        io::stdin().read_line(&mut api_hash)?;
        let api_hash = api_hash.trim().to_string();

        let config = Config {
            api_id,
            api_hash,
            phone_number: None,
            settings: Settings::default(),
            config_dir,
        };

        config.save()?;
        Ok(config)
    }

    fn get_config_dir() -> PathBuf {
        // First check current directory
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let local_config = current_dir.join("telegram_config.json");
        
        if local_config.exists() {
            return current_dir;
        }
        
        // Fall back to standard config locations
        if let Ok(config_dir) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(config_dir).join("telegram_client_rs")
        } else if let Some(home) = dirs::home_dir() {
            home.join(".config").join("telegram_client_rs")
        } else {
            PathBuf::from(".telegram_client_rs")
        }
    }

    pub fn session_path(&self) -> PathBuf {
        self.config_dir.join("telegram_session.session")
    }

    pub fn layout_path(&self) -> PathBuf {
        self.config_dir.join("telegram_layout.json")
    }

    pub fn aliases_path(&self) -> PathBuf {
        self.config_dir.join("telegram_aliases.json")
    }
}

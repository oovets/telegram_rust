use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::fs;
use std::io::Write;

pub fn supports_kitty_graphics() -> bool {
    std::env::var("KITTY_WINDOW_ID").is_ok()
        || std::env::var("TERM")
            .map(|t| t.to_lowercase().contains("kitty"))
            .unwrap_or(false)
}

pub fn clear_all_images() -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(b"\x1b_Ga=d,d=A,q=2\x1b\\")?;
    stdout.flush()?;
    Ok(())
}

pub fn render_png_at(path: &str, col: u16, row: u16, cols: u16, rows: u16) -> Result<()> {
    let bytes = fs::read(path)?;
    let encoded = STANDARD.encode(bytes);
    let mut stdout = std::io::stdout().lock();

    // Move cursor to target top-left cell (1-based for ANSI)
    let cursor = format!("\x1b[{};{}H", row.max(1), col.max(1));
    stdout.write_all(cursor.as_bytes())?;

    let chunks: Vec<&[u8]> = encoded.as_bytes().chunks(4096).collect();
    if chunks.is_empty() {
        stdout.write_all(b"\x1b_Ga=T,f=100,c=1,r=1,q=2,m=0;\x1b\\")?;
    } else {
        for (idx, chunk) in chunks.iter().enumerate() {
            let more = if idx + 1 < chunks.len() { 1 } else { 0 };
            let meta = if idx == 0 {
                format!(
                    "a=T,f=100,c={},r={},q=2,m={};",
                    cols.max(1),
                    rows.max(1),
                    more
                )
            } else {
                format!("m={};", more)
            };
            stdout.write_all(b"\x1b_G")?;
            stdout.write_all(meta.as_bytes())?;
            stdout.write_all(chunk)?;
            stdout.write_all(b"\x1b\\")?;
        }
    }

    stdout.flush()?;
    Ok(())
}

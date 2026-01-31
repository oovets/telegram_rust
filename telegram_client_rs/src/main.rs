use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

mod app;
mod commands;
mod config;
mod formatting;
mod persistence;
mod split_view;
mod telegram;
mod utils;
mod widgets;

use app::App;

#[tokio::main]
async fn main() -> Result<()> {
    // Create app BEFORE entering TUI mode (so authentication can work)
    let app = App::new().await?;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run app
    let res = run_app(&mut terminal, app).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("Error: {:?}", err);
    }

    Ok(())
}

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
) -> Result<()> {
    loop {
        terminal.draw(|f| app.draw(f))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            let event = event::read()?;
            match event {
                Event::Key(key) => {
                    match key.code {
                    // Ctrl+Q: Quit
                    KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.save_state()?;
                        break;
                    }
                    // Ctrl+R: Refresh chats
                    KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.refresh_chats().await?;
                    }
                    // Ctrl+V: Split vertical
                    KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.split_vertical();
                    }
                    // Ctrl+B: Split horizontal
                    KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.split_horizontal();
                    }
                    // Ctrl+W: Close pane
                    KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.close_pane();
                    }                    // Ctrl+S: Toggle chat list (Sidebar)
                    KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_chat_list();
                    }                    // Ctrl+L: Clear pane
                    KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.clear_pane();
                    }
                    // Ctrl+E: Toggle reactions
                    KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_reactions();
                    }
                    // Ctrl+N: Toggle notifications
                    KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_notifications();
                    }
                    // Ctrl+D: Toggle compact mode
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_compact();
                    }
                    // Ctrl+O: Toggle emojis
                    KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_emojis();
                    }
                    // Ctrl+G: Toggle line numbers
                    KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_line_numbers();
                    }
                    // Ctrl+T: Toggle timestamps
                    KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_timestamps();
                    }
                    // Esc: Cancel reply mode
                    KeyCode::Esc => {
                        if let Some(pane) = app.panes.get_mut(app.focused_pane_idx) {
                            if pane.reply_to_message.is_some() {
                                pane.reply_to_message = None;
                                pane.hide_reply_preview();
                            }
                        }
                    }                    // Tab: Autocomplete or cycle focus
                    KeyCode::Tab => {
                        app.handle_tab();
                    }
                    // Arrow keys
                    KeyCode::Up => {
                        app.handle_up();
                    }
                    KeyCode::Down => {
                        app.handle_down();
                    }
                    // PageUp/PageDown: Scroll messages
                    KeyCode::PageUp => {
                        app.handle_page_up();
                    }
                    KeyCode::PageDown => {
                        app.handle_page_down();
                    }
                    // Enter: Submit
                    KeyCode::Enter => {
                        app.handle_enter().await?;
                    }
                    // Character input (only when not on chat list)
                    KeyCode::Char(c) => {
                        if !app.focus_on_chat_list {
                            app.handle_char(c);
                        }
                    }
                    // Backspace
                    KeyCode::Backspace => {
                        if !app.focus_on_chat_list {
                            app.handle_backspace();
                        }
                    }
                    _ => {}
                    }
                }
                Event::Mouse(mouse) => {
                    if let event::MouseEventKind::Down(event::MouseButton::Left) = mouse.kind {
                        // Check if clicking on chat list first
                        if let Some(area) = app.chat_list_area {
                            if mouse.column >= area.x && mouse.column < area.x + area.width 
                                && mouse.row >= area.y && mouse.row < area.y + area.height {
                                // Clicked on chat list
                                app.handle_chat_list_click(mouse.row, area).await?;
                            }
                        }
                        // Check if clicking on a pane
                        app.handle_mouse_click(mouse.column, mouse.row);
                    }
                }
                Event::Resize(_, _) => {
                    // Terminal resize - ratatui handles this automatically on next draw
                }
                _ => {}
            }
        }

        // Process any Telegram events
        app.process_telegram_events().await?;
    }

    Ok(())
}

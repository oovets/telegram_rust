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
    let mut app = App::new().await?;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run app
    let res = run_app(&mut terminal, &mut app).await;
    
    // Save state before exiting (even if there was an error)
    let _ = app.save_state();

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| app.draw(f))?;

        // Process Telegram events FIRST - check for new messages frequently
        app.process_telegram_events().await?;

        if event::poll(std::time::Duration::from_millis(50))? {
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
                    // Ctrl+K: Toggle split direction
                    KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_split_direction();
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
                        // Load messages for focused pane if needed
                        app.load_pane_messages_if_needed(app.focused_pane_idx).await;
                    }
                }
                Event::Resize(_, _) => {
                    // Terminal resize - ratatui handles this automatically on next draw
                }
                _ => {}
            }
        } else {
            // No events, but still process Telegram updates (non-blocking)
            // This ensures we get new messages even when user isn't typing
            app.process_telegram_events().await?;
        }
    }

    Ok(())
}

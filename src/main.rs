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
mod kitty_preview;
mod persistence;
mod split_view;
mod telegram;
mod utils;
mod widgets;

use app::App;

#[tokio::main]
async fn main() -> Result<()> {
    let mut app = App::new().await?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let _res = run_app(&mut terminal, &mut app).await;
    app.close_inline_preview();
    
    let _ = app.save_state();

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
    let mut last_telegram_check = std::time::Instant::now();

    loop {
        if app.needs_redraw {
            terminal.draw(|f| app.draw(f))?;
            app.maybe_render_inline_preview();
            app.needs_redraw = false;
        }

        if last_telegram_check.elapsed() >= std::time::Duration::from_millis(500) {
            let had_updates = app.process_telegram_events().await?;
            last_telegram_check = std::time::Instant::now();
            if had_updates {
                app.needs_redraw = true;
            }
        }

        let poll_timeout = std::time::Duration::from_millis(500)
            .saturating_sub(last_telegram_check.elapsed())
            .max(std::time::Duration::from_millis(16));

        if event::poll(poll_timeout)? {
            let event = event::read()?;
            match event {
                Event::Key(key) => {
                    if app.inline_preview_path.is_some() {
                        match key.code {
                            KeyCode::Esc => {
                                app.close_inline_preview();
                                continue;
                            }
                            KeyCode::Char('+') | KeyCode::Char('=') => {
                                app.zoom_inline_preview_in();
                                continue;
                            }
                            KeyCode::Char('-') => {
                                app.zoom_inline_preview_out();
                                continue;
                            }
                            KeyCode::Char('n') | KeyCode::Right => {
                                app.preview_next_image().await?;
                                continue;
                            }
                            KeyCode::Char('p') | KeyCode::Left => {
                                app.preview_prev_image().await?;
                                continue;
                            }
                            _ => {}
                        }
                    }
                    app.needs_redraw = true;
                    match key.code {
                    KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.close_inline_preview();
                        app.save_state()?;
                        break;
                    }
                    KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.refresh_chats().await?;
                    }
                    KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.split_vertical();
                    }
                    KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.split_horizontal();
                    }
                    KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_split_direction();
                    }
                    KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.close_pane();
                    }
                    KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_chat_list();
                    }
                    KeyCode::Char('m') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_unread_count();
                    }
                    KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_mute_selected_chat();
                    }
                    KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.clear_pane();
                    }
                    KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_reactions();
                    }
                    KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_notifications();
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_compact();
                    }
                    KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_emojis();
                    }
                    KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_line_numbers();
                    }
                    KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_timestamps();
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_user_colors();
                    }
                    KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_borders();
                    }
                    KeyCode::Esc => {
                        if let Some(pane) = app.panes.get_mut(app.focused_pane_idx) {
                            if pane.reply_to_message.is_some() {
                                pane.reply_to_message = None;
                                pane.hide_reply_preview();
                            }
                        }
                    }
                    KeyCode::BackTab => {
                        let input_empty = app
                            .panes
                            .get(app.focused_pane_idx)
                            .map_or(true, |p| p.input_buffer.is_empty());
                        if app.focus_on_chat_list || input_empty {
                            app.cycle_focus_reverse();
                        }
                    }
                    KeyCode::Tab => {
                        app.handle_tab();
                    }
                    KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.resize_chat_list_narrower();
                    }
                    KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.resize_chat_list_wider();
                    }
                    KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
                        app.focus_prev_pane();
                    }
                    KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => {
                        app.focus_next_pane();
                    }
                    KeyCode::Up => {
                        app.handle_up();
                    }
                    KeyCode::Down => {
                        app.handle_down();
                    }
                    KeyCode::Left => {
                        if !app.focus_on_chat_list {
                            app.handle_input_left();
                        }
                    }
                    KeyCode::Right => {
                        if !app.focus_on_chat_list {
                            app.handle_input_right();
                        }
                    }
                    KeyCode::Home => {
                        if !app.focus_on_chat_list {
                            app.handle_home();
                        }
                    }
                    KeyCode::End => {
                        if !app.focus_on_chat_list {
                            app.handle_end();
                        }
                    }
                    KeyCode::PageUp => {
                        app.handle_page_up();
                    }
                    KeyCode::PageDown => {
                        app.handle_page_down();
                    }
                    KeyCode::Enter => {
                        app.handle_enter().await?;
                    }
                    KeyCode::Char(c) => {
                        if !app.focus_on_chat_list {
                            app.handle_char(c);
                        }
                    }
                    KeyCode::Backspace => {
                        if !app.focus_on_chat_list {
                            app.handle_backspace();
                        }
                    }
                    KeyCode::Delete => {
                        if !app.focus_on_chat_list {
                            app.handle_delete();
                        }
                    }
                    _ => {}
                    }
                }
                Event::Mouse(mouse) => {
                    app.needs_redraw = true;
                    if let event::MouseEventKind::Down(event::MouseButton::Left) = mouse.kind {
                        if let Some(area) = app.chat_list_area {
                            if mouse.column >= area.x && mouse.column < area.x + area.width 
                                && mouse.row >= area.y && mouse.row < area.y + area.height {
                                app.handle_chat_list_click(mouse.row, area).await?;
                            }
                        }
                        app.handle_mouse_click(mouse.column, mouse.row);
                        app.load_pane_messages_if_needed(app.focused_pane_idx).await;
                    }
                }
                Event::Resize(_, _) => {
                    app.needs_redraw = true;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

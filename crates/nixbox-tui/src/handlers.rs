use anyhow::Result;
use crossterm::event::{Event as CtEvent, KeyCode, KeyEventKind, KeyModifiers};
use nixbox_nix::build::BuildEvent;
use tokio::sync::mpsc;
use tui_input::backend::crossterm::EventHandler;

use crate::app::{App, AppEvent, Mode, SearchInputMode, Tab, CHANNELS};
use crate::nav::{
    cycle_tab, cycle_tab_back, move_installed_selection, move_selection, open_channel_edit,
    toggle_target, cycle_theme,
};
use crate::ops::{drain_queue, install_selected, migrate_all, migrate_selected, schedule_search, uninstall_selected};
use crate::theme;

pub(crate) async fn handle_terminal_event(
    app: &mut App,
    tx: &mpsc::Sender<AppEvent>,
    ev: CtEvent,
) -> Result<()> {
    let CtEvent::Key(key) = ev else { return Ok(()) };
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        app.should_quit = true;
        return Ok(());
    }

    if let Mode::ChannelEdit = app.mode {
        handle_channel_edit(app, key.code);
        return Ok(());
    }

    if let Mode::ThemeSelect = app.mode {
        handle_theme_select(app, key.code, key.modifiers);
        return Ok(());
    }

    if matches!(key.code, KeyCode::Esc)
        && matches!(app.tab, Tab::Search)
        && matches!(app.search_input_mode, SearchInputMode::Insert)
    {
        app.search_input_mode = SearchInputMode::Normal;
        return Ok(());
    }

    match key.code {
        KeyCode::Tab => { cycle_tab(app); return Ok(()); }
        KeyCode::BackTab => { cycle_tab_back(app); return Ok(()); }
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            toggle_target(app); return Ok(());
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            cycle_theme(app); return Ok(());
        }
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            open_channel_edit(app); return Ok(());
        }
        KeyCode::Esc => { app.should_quit = true; return Ok(()); }
        _ => {}
    }

    match app.tab {
        Tab::Search => match app.search_input_mode {
            SearchInputMode::Insert => match key.code {
                KeyCode::Down => move_selection(app, 1),
                KeyCode::Up => move_selection(app, -1),
                KeyCode::Enter => install_selected(app, tx).await?,
                _ => {
                    let before = app.input.value().to_string();
                    app.input.handle_event(&CtEvent::Key(key));
                    if app.input.value() != before {
                        schedule_search(app, tx.clone());
                    }
                }
            },
            SearchInputMode::Normal => match key.code {
                KeyCode::Down | KeyCode::Char('j') => move_selection(app, 1),
                KeyCode::Up | KeyCode::Char('k') => move_selection(app, -1),
                KeyCode::Char('l') => cycle_tab(app),
                KeyCode::Char('h') => cycle_tab_back(app),
                KeyCode::Enter => install_selected(app, tx).await?,
                KeyCode::Char('i') | KeyCode::Char('a') => {
                    app.search_input_mode = SearchInputMode::Insert;
                }
                _ => {}
            },
        },
        Tab::Installed => match key.code {
            KeyCode::Down | KeyCode::Char('j') => move_installed_selection(app, 1),
            KeyCode::Up | KeyCode::Char('k') => move_installed_selection(app, -1),
            KeyCode::Char('l') => cycle_tab(app),
            KeyCode::Char('h') => cycle_tab_back(app),
            KeyCode::Delete | KeyCode::Char('d') => uninstall_selected(app, tx).await?,
            KeyCode::Char('m') => migrate_selected(app, tx).await?,
            KeyCode::Char('M') => migrate_all(app, tx).await?,
            _ => {}
        },
        Tab::Building | Tab::Queue => match key.code {
            KeyCode::Char('l') => cycle_tab(app),
            KeyCode::Char('h') => cycle_tab_back(app),
            _ => {}
        },
    }
    Ok(())
}

fn handle_channel_edit(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Down | KeyCode::Char('j') => {
            app.channel_cursor = (app.channel_cursor + 1) % CHANNELS.len();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let n = CHANNELS.len();
            app.channel_cursor = app.channel_cursor.checked_sub(1).unwrap_or(n - 1);
        }
        KeyCode::Enter => {
            let new = CHANNELS[app.channel_cursor].to_string();
            app.config.channel = new.clone();
            let _ = app.config.save();
            app.status = format!("Channel set to {}.", new);
            app.mode = Mode::Browsing;
        }
        KeyCode::Esc => {
            app.mode = Mode::Browsing;
            app.status = format!("Channel unchanged: {}.", app.config.channel);
        }
        _ => {}
    }
}

fn handle_theme_select(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            let n = theme::ALL.len();
            app.theme_cursor = app.theme_cursor.checked_sub(1).unwrap_or(n - 1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.theme_cursor = (app.theme_cursor + 1) % theme::ALL.len();
        }
        KeyCode::Enter => {
            app.theme_index = app.theme_cursor;
            app.config.theme = theme::ALL[app.theme_index].name.to_string();
            let _ = app.config.save();
            app.status = format!("Theme set to {}.", theme::ALL[app.theme_index].name);
            app.mode = Mode::Browsing;
        }
        KeyCode::Esc => {
            app.mode = Mode::Browsing;
            app.status = format!("Theme: {}.", theme::ALL[app.theme_index].name);
        }
        KeyCode::Char('t') if modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = Mode::Browsing;
            app.status = format!("Theme: {}.", theme::ALL[app.theme_index].name);
        }
        _ => {}
    }
}

pub(crate) fn handle_app_event(app: &mut App, tx: &mpsc::Sender<AppEvent>, ev: AppEvent) {
    match ev {
        AppEvent::SearchDone { epoch, hits } => {
            if epoch == app.search_epoch {
                app.searching = false;
                let count = hits.len();
                app.results = hits;
                app.selected = 0;
                app.status = format!("{} matches for `{}`", count, app.latest_query);
            }
        }
        AppEvent::SearchFailed { epoch, error } => {
            if epoch == app.search_epoch {
                app.searching = false;
                app.results.clear();
                app.status = format!("search failed: {}", error);
            }
        }
        AppEvent::Build(BuildEvent::Line(line)) => {
            app.log.push(line);
            if app.log.len() > 1000 {
                let drop_to = app.log.len() - 1000;
                app.log.drain(0..drop_to);
            }
        }
        AppEvent::Build(BuildEvent::Finished(result)) => {
            let label = app.current_op_label.take().unwrap_or_else(|| "build".into());
            app.build_in_progress = false;
            app.status = match &result {
                Ok(()) => format!("{} done.", label),
                Err(err) => format!("{} failed: {}.", label, err),
            };
            if result.is_ok() {
                app.refresh_external_packages();
            }
            drain_queue(app, tx);
            if !app.visible_tabs().contains(&app.tab) {
                app.tab = Tab::Search;
            }
        }
    }
}


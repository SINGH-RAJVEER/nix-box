use nixbox_config::Target;

use crate::app::{App, Mode, SearchInputMode, Tab, CHANNELS};
use crate::theme;

pub(crate) fn move_selection(app: &mut App, delta: i32) {
    if app.results.is_empty() {
        return;
    }
    let len = app.results.len() as i32;
    let next = (app.selected as i32 + delta).rem_euclid(len);
    app.selected = next as usize;
}

pub(crate) fn move_installed_selection(app: &mut App, delta: i32) {
    let len = app.installed_total();
    if len == 0 {
        return;
    }
    let next = (app.installed_selected as i32 + delta).rem_euclid(len as i32);
    app.installed_selected = next as usize;
}

pub(crate) fn cycle_tab(app: &mut App) {
    let tabs = app.visible_tabs();
    let cur = tabs.iter().position(|t| *t == app.tab).unwrap_or(0);
    let next = tabs[(cur + 1) % tabs.len()];
    set_tab(app, next);
}

pub(crate) fn cycle_tab_back(app: &mut App) {
    let tabs = app.visible_tabs();
    let cur = tabs.iter().position(|t| *t == app.tab).unwrap_or(0);
    let prev = tabs[(cur + tabs.len() - 1) % tabs.len()];
    set_tab(app, prev);
}

pub(crate) fn set_tab(app: &mut App, tab: Tab) {
    app.tab = tab;
    match tab {
        Tab::Search => app.search_input_mode = SearchInputMode::Normal,
        Tab::Installed => app.installed_input_mode = SearchInputMode::Normal,
        _ => {}
    }
    app.status = format!("{} tab", tab.label());
}

pub(crate) fn toggle_target(app: &mut App) {
    app.config.target = match app.config.target {
        Target::HomeManager => Target::NixosSystem,
        Target::NixosSystem => Target::HomeManager,
    };
    let _ = app.config.save();
    app.status = format!(
        "Install target switched to {} (existing entries unchanged).",
        app.config.target.label()
    );
}

pub(crate) fn cycle_theme(app: &mut App) {
    app.theme_cursor = app.theme_index;
    app.mode = Mode::ThemeSelect;
    let _ = theme::ALL; // ensure theme module is used
    app.status = "↑/↓ preview  ·  Enter confirm  ·  Esc cancel".into();
}

pub(crate) fn open_channel_edit(app: &mut App) {
    app.channel_cursor = CHANNELS
        .iter()
        .position(|c| *c == app.config.channel)
        .unwrap_or(0);
    app.mode = Mode::ChannelEdit;
    app.status = "j/k navigate  ↵ confirm  esc cancel".into();
}

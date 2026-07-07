use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use super::{SPINNER, panel};
use crate::app::{App, Mode, SearchInputMode, Tab};

pub(super) fn draw_search_bar(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let dim = Style::default().add_modifier(Modifier::DIM);
    let (mode, value) = match app.tab {
        Tab::Installed => (app.installed_input_mode, app.installed_input.value()),
        _ => (app.search_input_mode, app.input.value()),
    };
    let (mode_label, mode_style) = match mode {
        SearchInputMode::Insert => (" INSERT ", t.title_style()),
        SearchInputMode::Normal => (" NORMAL ", dim),
    };
    let line = Line::from(vec![
        Span::styled(mode_label, mode_style),
        Span::styled("  /  ", dim),
        Span::raw(value.to_string()),
    ]);
    f.render_widget(Paragraph::new(line).block(panel(t)), area);
}

pub(super) fn draw_tab_strip(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let dim = Style::default().add_modifier(Modifier::DIM);
    let tabs = app.visible_tabs();

    let context_width = context_pills_width(app).min(area.width as usize) as u16;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(10), Constraint::Length(context_width)])
        .split(area);

    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    for tab in tabs.iter() {
        let is_active = *tab == app.tab;

        let mut label = tab.label().to_string();
        if matches!(tab, Tab::Building) && app.build_in_progress {
            let spin = SPINNER[app.spinner_frame % SPINNER.len()];
            label = format!("{} {}", spin, label);
        }
        if matches!(tab, Tab::Queue) && !app.queue.is_empty() {
            label = format!("{} ({})", label, app.queue.len());
        }

        let style = if is_active { t.selection_style() } else { dim };
        spans.push(Span::styled(format!(" {} ", label), style));
        spans.push(Span::raw("  "));
    }

    let context = Line::from(vec![
        Span::styled("channel ", dim),
        Span::styled(app.channel().to_string(), t.name_style()),
        Span::styled("  target ", dim),
        Span::styled(app.target_label().to_string(), t.title_style()),
        Span::raw(" "),
    ]);

    f.render_widget(Paragraph::new(Line::from(spans)), chunks[0]);
    f.render_widget(Paragraph::new(context), chunks[1]);
}

fn context_pills_width(app: &App) -> usize {
    // Keep channel/target visible without dedicating a full top bar to them.
    18 + app.channel().chars().count() + app.target_label().chars().count()
}

pub(super) fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let status = app.status.as_str();
    let keys = context_keys(app);

    let inner_width = area.width as usize;
    let keys_len = keys.chars().count();
    let status_chars = status.chars().count();
    let pad = inner_width.saturating_sub(status_chars + keys_len + 2);

    let line = Line::from(vec![
        Span::raw(status.to_string()),
        Span::raw(format!("  {:pad$}", "", pad = pad)),
        Span::styled(keys, Style::default().add_modifier(Modifier::DIM)),
    ]);

    let block = Block::default()
        .borders(Borders::TOP)
        .border_type(BorderType::Plain)
        .border_style(t.border_style());

    f.render_widget(Paragraph::new(line).block(block), area);
}

fn context_keys(app: &App) -> &'static str {
    match app.mode {
        Mode::ThemeSelect => "j/k preview  ↵ confirm  esc cancel",
        Mode::ChannelEdit => "↵ confirm  esc cancel",
        Mode::Browsing => match app.tab {
            Tab::Search => match app.search_input_mode {
                SearchInputMode::Insert => {
                    "↑↓ nav  ↵ install  ^g target  ^n channel  tab switch  esc normal"
                }
                SearchInputMode::Normal => {
                    "j/k nav  h/l tabs  ↵ install  i insert  ^g target  ^n channel  esc quit"
                }
            },
            Tab::Installed => match app.installed_input_mode {
                SearchInputMode::Insert => "↑↓ nav  tab switch  esc normal",
                SearchInputMode::Normal => {
                    "j/k nav  h/l tabs  d uninstall  m migrate  M migrate all  i filter  esc quit"
                }
            },
            Tab::Building => "h/l tabs  esc quit",
            Tab::Queue => "h/l tabs  esc quit",
        },
    }
}

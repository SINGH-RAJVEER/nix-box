use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, Mode};

pub(crate) fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(f.area());

    draw_header(f, chunks[0], app);

    match app.mode {
        Mode::Search => draw_search_body(f, chunks[1], app),
        Mode::Building => draw_build_body(f, chunks[1], app),
    }

    draw_footer(f, chunks[2], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let title = format!(
        "NixBox  ·  channel: {}  ·  target: {}  (Tab to toggle)",
        app.channel(),
        app.target_label()
    );
    let block = Block::default().borders(Borders::ALL).title(title);
    let p = Paragraph::new(format!("> {}", app.input.value())).block(block);
    f.render_widget(p, area);
}

fn draw_search_body(f: &mut Frame, area: Rect, app: &App) {
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    let items: Vec<ListItem> = app
        .results
        .iter()
        .map(|hit| {
            let line = Line::from(vec![
                Span::styled(
                    hit.pname.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(hit.version.clone(), Style::default().fg(Color::DarkGray)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Results"))
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    if !app.results.is_empty() {
        state.select(Some(app.selected));
    }
    f.render_stateful_widget(list, split[0], &mut state);

    let detail_block = Block::default().borders(Borders::ALL).title("Details");
    let detail_text = if let Some(hit) = app.results.get(app.selected) {
        format!(
            "{} {}\n\nattribute: {}\n\n{}\n\nEnter installs into {}.",
            hit.pname,
            hit.version,
            hit.attr,
            if hit.description.is_empty() {
                "(no description)"
            } else {
                hit.description.as_str()
            },
            app.target_label(),
        )
    } else {
        "Type to search nixpkgs. ↑/↓ select, Enter installs, Tab toggles target, Esc quits."
            .to_string()
    };
    let p = Paragraph::new(detail_text)
        .block(detail_block)
        .wrap(Wrap { trim: false });
    f.render_widget(p, split[1]);
}

fn draw_build_body(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("Build output");
    let height = area.height.saturating_sub(2) as usize;
    let start = app.log.len().saturating_sub(height);
    let text: Vec<Line> = app.log[start..]
        .iter()
        .map(|l| Line::from(l.clone()))
        .collect();
    let p = Paragraph::new(text).block(block);
    f.render_widget(p, area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("Status");
    let p = Paragraph::new(app.status.clone()).block(block);
    f.render_widget(p, area);
}

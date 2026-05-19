use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use super::{titled_panel, SPINNER};

pub(super) fn draw_search_body(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    let results_block = titled_panel(t, Span::styled("Results", t.title_style()));

    let no_results = !app.searching
        && !app.latest_query.is_empty()
        && app.results.is_empty();

    if app.searching && app.results.is_empty() {
        let frame = SPINNER[app.spinner_frame % SPINNER.len()];
        let inner_h = split[0].height.saturating_sub(2) as usize;
        let pad = inner_h / 2;
        let mut lines: Vec<Line> = (0..pad).map(|_| Line::raw("")).collect();
        lines.push(Line::from(vec![
            Span::styled(format!("  {}  ", frame), t.version_style()),
            Span::styled("Searching nixpkgs…", Style::default().add_modifier(Modifier::DIM)),
        ]));
        f.render_widget(Paragraph::new(lines).block(results_block), split[0]);
    } else if no_results {
        let dim = Style::default().add_modifier(Modifier::DIM);
        let inner_h = split[0].height.saturating_sub(2) as usize;
        let pad = inner_h / 2;
        let mut lines: Vec<Line> = (0..pad).map(|_| Line::raw("")).collect();
        lines.push(Line::from(vec![
            Span::styled("No results for ", dim),
            Span::styled(format!("\"{}\"", app.latest_query), t.name_style()),
        ]));
        f.render_widget(Paragraph::new(lines).block(results_block), split[0]);
    } else {
        let items: Vec<ListItem> = app
            .results
            .iter()
            .map(|hit| {
                ListItem::new(Line::from(vec![
                    Span::styled(hit.pname.clone(), t.name_style()),
                    Span::raw("  "),
                    Span::styled(hit.version.clone(), t.version_style()),
                ]))
            })
            .collect();

        let list = List::new(items)
            .block(results_block)
            .highlight_style(t.selection_style())
            .highlight_symbol("❯ ");

        let mut state = ListState::default();
        if !app.results.is_empty() {
            state.select(Some(app.selected));
        }
        f.render_stateful_widget(list, split[0], &mut state);
    }

    let detail_block = titled_panel(t, Span::styled("Details", t.title_style()));
    let sep_w = split[1].width.saturating_sub(2) as usize;
    let sep: String = "─".repeat(sep_w);
    let dim = Style::default().add_modifier(Modifier::DIM);

    if let Some(hit) = app.results.get(app.selected) {
        let desc = if hit.description.is_empty() {
            "(no description)".to_string()
        } else {
            hit.description.clone()
        };
        let lines: Vec<Line> = vec![
            Line::from(Span::styled(hit.pname.clone(), t.name_style())),
            Line::from(Span::styled(hit.version.clone(), t.version_style())),
            Line::raw(""),
            Line::from(Span::styled(sep.clone(), dim)),
            Line::raw(""),
            Line::from(Span::styled("Attribute", dim)),
            Line::from(Span::raw(hit.attr.clone())),
            Line::raw(""),
            Line::from(Span::styled("Description", dim)),
            Line::from(Span::raw(desc)),
            Line::raw(""),
            Line::from(Span::styled(sep.clone(), dim)),
            Line::raw(""),
            Line::from(Span::styled(format!("↵  install → {}", app.target_label()), dim)),
        ];
        f.render_widget(
            Paragraph::new(lines).block(detail_block).wrap(Wrap { trim: false }),
            split[1],
        );
    } else {
        let lines: Vec<Line> = vec![
            Line::from(Span::styled("nixpkgs search", t.title_style())),
            Line::raw(""),
            Line::from(Span::styled(sep, dim)),
            Line::raw(""),
            Line::from(Span::styled("/  type to search", dim)),
            Line::from(Span::styled("↑↓ j/k  navigate", dim)),
            Line::from(Span::styled("↵   install", dim)),
            Line::from(Span::styled("h/l  switch tabs", dim)),
        ];
        f.render_widget(
            Paragraph::new(lines).block(detail_block).wrap(Wrap { trim: false }),
            split[1],
        );
    }
}

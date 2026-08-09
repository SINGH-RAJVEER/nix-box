use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};

use super::{SPINNER, titled_panel};
use crate::app::App;

pub(super) fn draw_flakes_body(f: &mut Frame, area: Rect, app: &App) {
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(area);
    draw_results(f, split[0], app);
    draw_details(f, split[1], app);
}

fn draw_results(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let block = titled_panel(t, Span::styled("GitHub flakes", t.title_style()));
    let dim = Style::default().add_modifier(Modifier::DIM);
    if app.flake_searching && app.flake_results.is_empty() {
        let spinner = SPINNER[app.spinner_frame % SPINNER.len()];
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {} ", spinner), t.version_style()),
                Span::styled("Searching GitHub code…", dim),
            ]))
            .block(block),
            area,
        );
        return;
    }
    if app.flake_results.is_empty() {
        let message = if app.flake_query.is_empty() {
            "Type a project name, input, or output to search root flake.nix files."
        } else {
            "No root flake.nix files matched this query."
        };
        f.render_widget(
            Paragraph::new(message)
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = app
        .flake_results
        .iter()
        .map(|hit| ListItem::new(Line::from(Span::styled(hit.repo.clone(), t.name_style()))))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(t.selection_style())
        .highlight_symbol("❯ ");
    let mut state = ListState::default();
    state.select(Some(app.flake_selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_details(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let block = titled_panel(t, Span::styled("Flake properties", t.title_style()));
    let dim = Style::default().add_modifier(Modifier::DIM);
    let sep = "─".repeat(area.width.saturating_sub(2) as usize);
    if app.flake_detail_loading {
        let spinner = SPINNER[app.spinner_frame % SPINNER.len()];
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {} ", spinner), t.version_style()),
                Span::styled("Reading repository and flake.nix…", dim),
            ]))
            .block(block),
            area,
        );
        return;
    }
    let Some(details) = &app.flake_details else {
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("GitHub flake browser", t.title_style())),
                Line::raw(""),
                Line::from(Span::styled(sep, dim)),
                Line::raw(""),
                Line::from(Span::styled("Searches only root flake.nix files.", dim)),
                Line::from(Span::styled("Results are not cloned or persisted.", dim)),
                Line::raw(""),
                Line::from(Span::styled(
                    "i  search    j/k  select    Enter  install module",
                    dim,
                )),
            ])
            .block(block)
            .wrap(Wrap { trim: false }),
            area,
        );
        return;
    };

    let description = details.description.as_deref().unwrap_or("(no description)");
    let inputs = list_or_none(&details.inputs);
    let outputs = list_or_none(&details.outputs);
    let topics = list_or_none(&details.topics);
    let pushed = details.pushed_at.as_deref().unwrap_or("unknown");
    let homepage = details.homepage.as_deref().unwrap_or("none");
    let status = if details.archived {
        "archived"
    } else {
        "active"
    };
    let lines = vec![
        Line::from(Span::styled(details.repo.clone(), t.name_style())),
        Line::from(Span::styled(
            format!("★ {}  ·  {}", details.stars, status),
            t.version_style(),
        )),
        Line::raw(""),
        Line::from(Span::styled(sep, dim)),
        Line::raw(""),
        Line::from(Span::styled("Description", dim)),
        Line::from(description.to_string()),
        Line::raw(""),
        Line::from(Span::styled("Flake", dim)),
        Line::from(details.path.clone()),
        Line::from(Span::styled("Inputs", dim)),
        Line::from(inputs),
        Line::from(Span::styled("Outputs", dim)),
        Line::from(outputs),
        Line::from(Span::styled("Topics", dim)),
        Line::from(topics),
        Line::raw(""),
        Line::from(Span::styled("Repository", dim)),
        Line::from(details.repo_url.clone()),
        Line::from(Span::styled(
            format!("branch {}  ·  pushed {}", details.default_branch, pushed),
            dim,
        )),
        Line::from(Span::styled(format!("homepage {}", homepage), dim)),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn list_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "(not detected)".to_string()
    } else {
        values.join("  ·  ")
    }
}

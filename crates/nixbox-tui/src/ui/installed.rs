use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use super::titled_panel;

pub(super) fn draw_installed_body(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let managed = app.installed_packages();
    let external = &app.external_packages;

    if managed.is_empty() && external.is_empty() {
        let block = titled_panel(t, Span::styled("Installed packages", t.title_style()));
        f.render_widget(
            Paragraph::new(
                "No packages tracked yet.\n\nSwitch to the Search tab and press ↵ on a result to install.",
            )
            .block(block)
            .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }

    let dim = Style::default().add_modifier(Modifier::DIM);

    let mut items: Vec<ListItem> = Vec::new();
    let mut pkg_to_row: Vec<usize> = Vec::new();

    if !managed.is_empty() {
        let header = format!(" Managed  ({}) ", managed.len());
        items.push(ListItem::new(Line::from(Span::styled(header, dim))));
        for p in &managed {
            pkg_to_row.push(items.len());
            items.push(ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(p.clone(), t.name_style()),
            ])));
        }
    }

    if !external.is_empty() {
        if !managed.is_empty() {
            items.push(ListItem::new(Line::raw("")));
        }
        let migratable_count = external.iter().filter(|ep| ep.migratable).count();
        let header = format!(
            " External  ({}, {} migratable) ",
            external.len(),
            migratable_count,
        );
        items.push(ListItem::new(Line::from(vec![
            Span::styled(header, dim),
            Span::styled("  m migrate  M migrate all", dim),
        ])));
        for ep in external {
            pkg_to_row.push(items.len());
            let name_style = if ep.migratable {
                t.name_style()
            } else {
                t.name_style().add_modifier(Modifier::DIM)
            };
            let mut spans = vec![
                Span::raw("  "),
                Span::styled(ep.name.clone(), name_style),
                Span::raw("  "),
                Span::styled(format!("({})", ep.source_attr), dim),
            ];
            if !ep.migratable {
                spans.push(Span::styled("  · inline", dim));
            }
            items.push(ListItem::new(Line::from(spans)));
        }
    }

    let title = Span::styled("Installed packages", t.title_style());
    let list = List::new(items)
        .block(titled_panel(t, title))
        .highlight_style(t.selection_style())
        .highlight_symbol("❯ ");

    let selected_row = if pkg_to_row.is_empty() {
        None
    } else {
        let clamped = app.installed_selected.min(pkg_to_row.len() - 1);
        Some(pkg_to_row[clamped])
    };

    let mut state = ListState::default();
    state.select(selected_row);
    f.render_stateful_widget(list, area, &mut state);
}

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState};
use ratatui::Frame;

use crate::app::{App, CHANNELS};
use crate::theme;
use super::titled_panel;

pub(super) fn draw_theme_popup(f: &mut Frame, app: &App) {
    let t = app.theme();
    let area = f.area();
    let popup_width: u16 = 36;
    let popup_height: u16 = theme::ALL.len() as u16 + 2;
    let x = area.x + area.width.saturating_sub(popup_width) / 2;
    let y = area.y + area.height.saturating_sub(popup_height) / 2;
    let popup_area = Rect::new(x, y, popup_width.min(area.width), popup_height.min(area.height));

    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = theme::ALL
        .iter()
        .enumerate()
        .map(|(i, th)| {
            let check = if i == app.theme_index { "  ✓" } else { "" };
            ListItem::new(Line::from(Span::styled(
                format!("  {}{}", th.name, check),
                t.name_style(),
            )))
        })
        .collect();

    let list = List::new(items)
        .block(titled_panel(t, Span::styled(" Select Theme ", t.title_style())))
        .highlight_style(t.selection_style())
        .highlight_symbol("❯");

    let mut state = ListState::default();
    state.select(Some(app.theme_cursor));
    f.render_stateful_widget(list, popup_area, &mut state);
}

pub(super) fn draw_channel_popup(f: &mut Frame, app: &App) {
    let t = app.theme();
    let area = f.area();
    let popup_width: u16 = 36;
    let popup_height: u16 = CHANNELS.len() as u16 + 2;
    let x = area.x + area.width.saturating_sub(popup_width) / 2;
    let y = area.y + area.height.saturating_sub(popup_height) / 2;
    let popup_area = Rect::new(x, y, popup_width.min(area.width), popup_height.min(area.height));

    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = CHANNELS
        .iter()
        .map(|ch| {
            let check = if *ch == app.config.channel { "  ✓" } else { "" };
            ListItem::new(Line::from(Span::styled(
                format!("  {}{}", ch, check),
                t.name_style(),
            )))
        })
        .collect();

    let list = List::new(items)
        .block(titled_panel(t, Span::styled(" Select Channel ", t.title_style())))
        .highlight_style(t.selection_style())
        .highlight_symbol("❯");

    let mut state = ListState::default();
    state.select(Some(app.channel_cursor));
    f.render_stateful_widget(list, popup_area, &mut state);
}

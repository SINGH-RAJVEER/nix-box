use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::theme;
use super::titled_panel;

pub(super) fn draw_build_body(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let title = match app.current_op_label.as_ref() {
        Some(label) => format!("Build output  ·  {}", label),
        None => "Build output".to_string(),
    };
    let block = titled_panel(t, Span::styled(title, t.title_style()));
    let inner_width = area.width.saturating_sub(2);
    let bar_lines: usize = if app.build_in_progress { 2 } else { 0 };
    let log_height = area.height.saturating_sub(2).saturating_sub(bar_lines as u16) as usize;
    let start = app.log.len().saturating_sub(log_height);

    let mut text: Vec<Line> = Vec::new();
    if app.build_in_progress {
        text.push(build_progress_bar(app.spinner_frame, inner_width, t));
        text.push(Line::raw(""));
    }
    text.extend(app.log[start..].iter().map(|l| Line::from(l.clone())));
    f.render_widget(Paragraph::new(text).block(block), area);
}

fn build_progress_bar(frame: usize, width: u16, t: &theme::Theme) -> Line<'static> {
    let w = width as usize;
    if w < 4 {
        return Line::raw("");
    }
    let block_size = (w / 5).max(6).min(w);
    let range = w.saturating_sub(block_size);
    let pos = if range == 0 {
        0
    } else {
        let cycle = 2 * range;
        let f = frame % cycle;
        if f <= range { f } else { cycle - f }
    };
    let before = "░".repeat(pos);
    let after = "░".repeat(w.saturating_sub(pos + block_size));
    let filled = if block_size >= 4 {
        let core = "█".repeat(block_size.saturating_sub(4));
        format!("▒▓{}▓▒", core)
    } else {
        "█".repeat(block_size)
    };
    let dim = Style::default().add_modifier(Modifier::DIM);
    Line::from(vec![
        Span::styled(before, dim),
        Span::styled(filled, t.title_style()),
        Span::styled(after, dim),
    ])
}

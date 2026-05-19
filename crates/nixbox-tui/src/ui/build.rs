use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use super::titled_panel;

pub(super) fn draw_build_body(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let title = match app.current_op_label.as_ref() {
        Some(label) => format!("Build output  ·  {}", label),
        None => "Build output".to_string(),
    };
    let block = titled_panel(t, Span::styled(title, t.title_style()));
    let log_height = area.height.saturating_sub(2) as usize;
    let start = app.log.len().saturating_sub(log_height);
    let text: Vec<Line> = app.log[start..].iter().map(|l| Line::from(l.clone())).collect();
    f.render_widget(Paragraph::new(text).block(block), area);
}

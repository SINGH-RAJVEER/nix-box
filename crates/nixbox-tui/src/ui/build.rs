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
        text.push(match app.build_progress {
            Some(pct) => build_fill_bar(pct, inner_width, t),
            None => build_indeterminate_bar(app.spinner_frame, inner_width, t),
        });
        text.push(Line::raw(""));
    }
    text.extend(app.log[start..].iter().map(|l| Line::from(l.clone())));
    f.render_widget(Paragraph::new(text).block(block), area);
}

fn build_fill_bar(pct: f32, width: u16, t: &theme::Theme) -> Line<'static> {
    let w = width as usize;
    if w < 4 {
        return Line::raw("");
    }
    let filled_cells = ((pct * w as f32).round() as usize).min(w);
    let empty_cells = w.saturating_sub(filled_cells);
    let pct_label = format!(" {:3.0}% ", pct * 100.0);
    let label_start = (w.saturating_sub(pct_label.len())) / 2;
    let label_end = label_start + pct_label.len();

    // Build each half of the bar directly, splicing in the label in-place.
    let mut filled = String::with_capacity(filled_cells * 3);
    let mut empty = String::with_capacity(empty_cells * 3);
    for i in 0..w {
        let ch = if i >= label_start && i < label_end {
            pct_label.as_bytes()[i - label_start] as char
        } else if i < filled_cells {
            '█'
        } else {
            '░'
        };
        if i < filled_cells {
            filled.push(ch);
        } else {
            empty.push(ch);
        }
    }
    let dim = Style::default().add_modifier(Modifier::DIM);
    Line::from(vec![
        Span::styled(filled, t.title_style()),
        Span::styled(empty, dim),
    ])
}

fn build_indeterminate_bar(frame: usize, width: u16, t: &theme::Theme) -> Line<'static> {
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

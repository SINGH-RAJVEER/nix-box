use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};
use ratatui::Frame;

use crate::app::{App, Mode, SearchInputMode, Tab};
use crate::theme;

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Rounded bordered block styled with the active theme.
fn panel<'a>(t: &theme::Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(t.border_type())
        .border_style(t.border_style())
}

fn titled_panel<'a>(t: &theme::Theme, title: Span<'a>) -> Block<'a> {
    panel(t).title(title)
}

pub(crate) fn draw(f: &mut Frame, app: &App) {
    if let Some(bg) = app.theme().bg_color {
        f.render_widget(Block::default().style(Style::default().bg(bg)), f.area());
    }

    let show_search_bar = matches!(app.tab, Tab::Search);

    let mut constraints = vec![
        Constraint::Length(3), // info bar
        Constraint::Length(1), // tab strip (borderless, 1 line) — always at fixed height
    ];
    if show_search_bar {
        constraints.push(Constraint::Length(3)); // search input
    }
    constraints.push(Constraint::Min(8));    // body
    constraints.push(Constraint::Length(2)); // footer (top border only)

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(f.area());

    let mut i = 0;
    draw_info_bar(f, chunks[i], app); i += 1;
    draw_tab_strip(f, chunks[i], app); i += 1;
    if show_search_bar {
        draw_search_bar(f, chunks[i], app); i += 1;
    }
    let body_area = chunks[i]; i += 1;
    let footer_area = chunks[i];

    match app.tab {
        Tab::Search => draw_search_body(f, body_area, app),
        Tab::Installed => draw_installed_body(f, body_area, app),
        Tab::Building => draw_build_body(f, body_area, app),
        Tab::Queue => draw_queue_body(f, body_area, app),
    }

    draw_footer(f, footer_area, app);

    if matches!(app.mode, Mode::ThemeSelect) {
        draw_theme_popup(f, app);
    }
    if matches!(app.mode, Mode::ChannelEdit) {
        draw_channel_popup(f, app);
    }
}

fn draw_info_bar(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let dim = Style::default().add_modifier(Modifier::DIM);
    let line = Line::from(vec![
        Span::styled("channel", dim),
        Span::raw("  "),
        Span::styled(app.channel().to_string(), t.name_style()),
        Span::styled("     │     ", dim),
        Span::styled("target", dim),
        Span::raw("  "),
        Span::styled(app.target_label().to_string(), t.name_style()),
    ]);
    f.render_widget(Paragraph::new(line).block(panel(t)), area);
}

fn draw_search_bar(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let dim = Style::default().add_modifier(Modifier::DIM);
    let (mode_label, mode_style) = match app.search_input_mode {
        SearchInputMode::Insert => (" INSERT ", t.title_style()),
        SearchInputMode::Normal => (" NORMAL ", dim),
    };
    let line = Line::from(vec![
        Span::styled(mode_label, mode_style),
        Span::styled("  /  ", dim),
        Span::raw(app.input.value().to_string()),
    ]);
    f.render_widget(Paragraph::new(line).block(panel(t)), area);
}

fn draw_tab_strip(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let dim = Style::default().add_modifier(Modifier::DIM);
    let tabs = app.visible_tabs();

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

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_search_body(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    let results_block = titled_panel(t, Span::styled("Results", t.title_style()));

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


fn draw_installed_body(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let block = titled_panel(t, Span::styled("Installed packages", t.title_style()));
    let packages = app.installed_packages();
    if packages.is_empty() {
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
    let items: Vec<ListItem> = packages
        .iter()
        .map(|p| ListItem::new(Line::from(Span::styled(p.clone(), t.name_style()))))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(t.selection_style())
        .highlight_symbol("❯ ");
    let mut state = ListState::default();
    state.select(Some(app.installed_selected.min(packages.len() - 1)));
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_queue_body(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let title = format!("Queue  ·  {} pending", app.queue.len());
    let block = titled_panel(t, Span::styled(title, t.title_style()));
    if app.queue.is_empty() {
        f.render_widget(
            Paragraph::new("Queue is empty.").block(block),
            area,
        );
        return;
    }
    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut lines: Vec<Line> = Vec::new();
    if let Some(running) = app.current_op_label.as_ref() {
        let spin = SPINNER[app.spinner_frame % SPINNER.len()];
        lines.push(Line::from(vec![
            Span::styled(format!("{}  ", spin), t.title_style()),
            Span::styled(running.clone(), t.name_style()),
            Span::styled("  running", dim),
        ]));
        lines.push(Line::raw(""));
    }
    for (i, op) in app.queue.iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(format!("{:>2}.  ", i + 1), dim),
            Span::styled(op.label(), t.name_style()),
        ]));
    }
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

fn draw_build_body(f: &mut Frame, area: Rect, app: &App) {
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

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
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
                SearchInputMode::Insert => "↑↓ nav  ↵ install  ^g target  ^n channel  tab switch  esc normal",
                SearchInputMode::Normal => "j/k nav  h/l tabs  ↵ install  i insert  ^g target  ^n channel  esc quit",
            },
            Tab::Installed => "j/k nav  h/l tabs  d uninstall  ^g target  ^n channel  esc quit",
            Tab::Building => "h/l tabs  esc quit",
            Tab::Queue => "h/l tabs  esc quit",
        },
    }
}

fn draw_theme_popup(f: &mut Frame, app: &App) {
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

fn draw_channel_popup(f: &mut Frame, app: &App) {
    use crate::app::CHANNELS;

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

use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use anyhow::Result;
use nixbox_config::{Config, Target};
use nixbox_nix::{
    build::{home_manager_switch_cmd, rebuild, BuildEvent},
    manifest::ManagedFile,
    search::{search, SearchHit},
    Manifest,
};
use crossterm::event::{Event as CtEvent, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use crate::theme;
use crate::ui;

pub(crate) const CHANNELS: &[&str] = &["nixpkgs", "nixpkgs-unstable"];

#[derive(Debug, Clone)]
pub(crate) enum Mode {
    Browsing,
    ThemeSelect,
    ChannelEdit,
}

#[derive(Debug, Clone)]
pub(crate) enum QueuedOp {
    Install(SearchHit),
    Uninstall(String),
}

impl QueuedOp {
    pub(crate) fn label(&self) -> String {
        match self {
            QueuedOp::Install(hit) => format!("install {}", hit.attr),
            QueuedOp::Uninstall(pname) => format!("remove {}", pname),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchInputMode {
    Insert,
    Normal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Search,
    Installed,
    Building,
    Queue,
}

impl Tab {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Tab::Search => "Search",
            Tab::Installed => "Installed",
            Tab::Building => "Building",
            Tab::Queue => "Queue",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum AppEvent {
    SearchDone { epoch: u64, hits: Vec<SearchHit> },
    SearchFailed { epoch: u64, error: String },
    Build(BuildEvent),
}

pub(crate) struct App {
    pub(crate) config: Config,
    pub(crate) manifest: Manifest,
    pub(crate) input: Input,
    pub(crate) results: Vec<SearchHit>,
    pub(crate) selected: usize,
    pub(crate) installed_selected: usize,
    pub(crate) status: String,
    pub(crate) mode: Mode,
    pub(crate) tab: Tab,
    pub(crate) log: Vec<String>,
    pub(crate) search_epoch: u64,
    pub(crate) latest_query: String,
    pub(crate) should_quit: bool,
    pub(crate) theme_index: usize,
    pub(crate) theme_cursor: usize,
    pub(crate) search_input_mode: SearchInputMode,
    pub(crate) searching: bool,
    pub(crate) build_in_progress: bool,
    pub(crate) spinner_frame: usize,
    pub(crate) queue: VecDeque<QueuedOp>,
    pub(crate) current_op_label: Option<String>,
    pub(crate) channel_cursor: usize,
}

impl App {
    fn new(config: Config, manifest: Manifest) -> Self {
        let installed = manifest.packages.len();
        let theme_index = theme::ALL
            .iter()
            .position(|t| t.name == config.theme)
            .unwrap_or(0);
        Self {
            config,
            manifest,
            input: Input::default(),
            results: Vec::new(),
            selected: 0,
            installed_selected: 0,
            status: format!("{} packages tracked.", installed),
            mode: Mode::Browsing,
            tab: Tab::Search,
            log: Vec::new(),
            search_epoch: 0,
            latest_query: String::new(),
            should_quit: false,
            theme_index,
            theme_cursor: theme_index,
            search_input_mode: SearchInputMode::Normal,
            searching: false,
            build_in_progress: false,
            spinner_frame: 0,
            queue: VecDeque::new(),
            current_op_label: None,
            channel_cursor: 0,
        }
    }

    pub(crate) fn visible_tabs(&self) -> Vec<Tab> {
        let mut tabs = vec![Tab::Search, Tab::Installed];
        if self.build_in_progress {
            tabs.push(Tab::Building);
        }
        if !self.queue.is_empty() {
            tabs.push(Tab::Queue);
        }
        tabs
    }

    pub(crate) fn channel(&self) -> &str {
        &self.config.channel
    }

    pub(crate) fn target_label(&self) -> &'static str {
        self.config.target.label()
    }

    pub(crate) fn theme(&self) -> &'static theme::Theme {
        let idx = if matches!(self.mode, Mode::ThemeSelect) {
            self.theme_cursor
        } else {
            self.theme_index
        };
        &theme::ALL[idx]
    }

    pub(crate) fn installed_packages(&self) -> Vec<String> {
        self.manifest.packages.iter().cloned().collect()
    }
}

pub async fn run() -> Result<()> {
    let config = Config::load_or_default()?;
    let manifest = ManagedFile::new(config.managed_file()).load()?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, config, manifest).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: Config,
    manifest: Manifest,
) -> Result<()> {
    let mut app = App::new(config, manifest);
    let (tx, mut rx) = mpsc::channel::<AppEvent>(128);
    let mut term_events = EventStream::new();
    let mut spinner_tick = tokio::time::interval(Duration::from_millis(80));
    spinner_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;
        if app.should_quit {
            break;
        }

        tokio::select! {
            Some(Ok(ev)) = term_events.next() => {
                handle_terminal_event(&mut app, &tx, ev).await?;
            }
            Some(app_ev) = rx.recv() => {
                handle_app_event(&mut app, &tx, app_ev);
            }
            _ = spinner_tick.tick(), if app.searching || app.build_in_progress => {
                app.spinner_frame = app.spinner_frame.wrapping_add(1);
            }
        }
    }
    Ok(())
}

async fn handle_terminal_event(
    app: &mut App,
    tx: &mpsc::Sender<AppEvent>,
    ev: CtEvent,
) -> Result<()> {
    let CtEvent::Key(key) = ev else { return Ok(()) };
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        app.should_quit = true;
        return Ok(());
    }

    if let Mode::ChannelEdit = app.mode {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                app.channel_cursor = (app.channel_cursor + 1) % CHANNELS.len();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let n = CHANNELS.len();
                app.channel_cursor = app.channel_cursor.checked_sub(1).unwrap_or(n - 1);
            }
            KeyCode::Enter => {
                let new = CHANNELS[app.channel_cursor].to_string();
                app.config.channel = new.clone();
                let _ = app.config.save();
                app.status = format!("Channel set to {}.", new);
                app.mode = Mode::Browsing;
            }
            KeyCode::Esc => {
                app.mode = Mode::Browsing;
                app.status = format!("Channel unchanged: {}.", app.config.channel);
            }
            _ => {}
        }
        return Ok(());
    }

    if let Mode::ThemeSelect = app.mode {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let n = theme::ALL.len();
                app.theme_cursor = app.theme_cursor.checked_sub(1).unwrap_or(n - 1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.theme_cursor = (app.theme_cursor + 1) % theme::ALL.len();
            }
            KeyCode::Enter => {
                app.theme_index = app.theme_cursor;
                app.config.theme = theme::ALL[app.theme_index].name.to_string();
                let _ = app.config.save();
                app.status = format!("Theme set to {}.", theme::ALL[app.theme_index].name);
                app.mode = Mode::Browsing;
            }
            KeyCode::Esc => {
                app.mode = Mode::Browsing;
                app.status = format!("Theme: {}.", theme::ALL[app.theme_index].name);
            }
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.mode = Mode::Browsing;
                app.status = format!("Theme: {}.", theme::ALL[app.theme_index].name);
            }
            _ => {}
        }
        return Ok(());
    }

    // Esc in Search Insert mode → Normal mode instead of quitting
    if matches!(key.code, KeyCode::Esc)
        && matches!(app.tab, Tab::Search)
        && matches!(app.search_input_mode, SearchInputMode::Insert)
    {
        app.search_input_mode = SearchInputMode::Normal;
        return Ok(());
    }

    // Global keys (work on any tab)
    match key.code {
        KeyCode::Tab => {
            cycle_tab(app);
            return Ok(());
        }
        KeyCode::BackTab => {
            cycle_tab_back(app);
            return Ok(());
        }
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            toggle_target(app);
            return Ok(());
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            cycle_theme(app);
            return Ok(());
        }
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            open_channel_edit(app);
            return Ok(());
        }
        KeyCode::Esc => {
            app.should_quit = true;
            return Ok(());
        }
        _ => {}
    }

    match app.tab {
        Tab::Search => match app.search_input_mode {
            SearchInputMode::Insert => match key.code {
                KeyCode::Down => move_selection(app, 1),
                KeyCode::Up => move_selection(app, -1),
                KeyCode::Enter => install_selected(app, tx).await?,
                _ => {
                    let before = app.input.value().to_string();
                    app.input.handle_event(&CtEvent::Key(key));
                    if app.input.value() != before {
                        schedule_search(app, tx.clone());
                    }
                }
            },
            SearchInputMode::Normal => match key.code {
                KeyCode::Down | KeyCode::Char('j') => move_selection(app, 1),
                KeyCode::Up | KeyCode::Char('k') => move_selection(app, -1),
                KeyCode::Char('l') => cycle_tab(app),
                KeyCode::Char('h') => cycle_tab_back(app),
                KeyCode::Enter => install_selected(app, tx).await?,
                KeyCode::Char('i') | KeyCode::Char('a') => {
                    app.search_input_mode = SearchInputMode::Insert;
                }
                _ => {}
            },
        },
        Tab::Installed => match key.code {
            KeyCode::Down | KeyCode::Char('j') => move_installed_selection(app, 1),
            KeyCode::Up | KeyCode::Char('k') => move_installed_selection(app, -1),
            KeyCode::Char('l') => cycle_tab(app),
            KeyCode::Char('h') => cycle_tab_back(app),
            KeyCode::Delete | KeyCode::Char('d') => uninstall_selected(app, tx).await?,
            _ => {}
        },
        Tab::Building => match key.code {
            KeyCode::Char('l') => cycle_tab(app),
            KeyCode::Char('h') => cycle_tab_back(app),
            _ => {}
        },
        Tab::Queue => match key.code {
            KeyCode::Char('l') => cycle_tab(app),
            KeyCode::Char('h') => cycle_tab_back(app),
            _ => {}
        },
    }
    Ok(())
}

fn handle_app_event(app: &mut App, tx: &mpsc::Sender<AppEvent>, ev: AppEvent) {
    match ev {
        AppEvent::SearchDone { epoch, hits } => {
            if epoch == app.search_epoch {
                app.searching = false;
                let count = hits.len();
                app.results = hits;
                app.selected = 0;
                app.status = format!("{} matches for `{}`", count, app.latest_query);
            }
        }
        AppEvent::SearchFailed { epoch, error } => {
            if epoch == app.search_epoch {
                app.searching = false;
                app.results.clear();
                app.status = format!("search failed: {}", error);
            }
        }
        AppEvent::Build(BuildEvent::Line(line)) => {
            app.log.push(line);
            if app.log.len() > 1000 {
                let drop_to = app.log.len() - 1000;
                app.log.drain(0..drop_to);
            }
        }
        AppEvent::Build(BuildEvent::Finished(result)) => {
            let label = app.current_op_label.take().unwrap_or_else(|| "build".into());
            app.build_in_progress = false;
            app.status = match &result {
                Ok(()) => format!("{} done.", label),
                Err(err) => format!("{} failed: {}.", label, err),
            };
            drain_queue(app, tx);
            if !app.visible_tabs().contains(&app.tab) {
                app.tab = Tab::Search;
            }
        }
    }
}

fn drain_queue(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    if app.build_in_progress {
        return;
    }
    let Some(op) = app.queue.pop_front() else { return };
    let label = op.label();
    if let Err(e) = apply_op_to_manifest(app, &op) {
        app.status = format!("{}: failed to write manifest: {}", label, e);
        // Try the next one rather than getting stuck.
        drain_queue(app, tx);
        return;
    }
    if !app.visible_tabs().contains(&app.tab) {
        // shouldn't happen but be safe
        app.tab = Tab::Search;
    }
    spawn_rebuild(app, tx, label);
}

fn apply_op_to_manifest(app: &mut App, op: &QueuedOp) -> Result<()> {
    match op {
        QueuedOp::Install(hit) => {
            app.manifest.add(&hit.attr);
        }
        QueuedOp::Uninstall(pname) => {
            app.manifest.remove(pname);
            if app.installed_selected >= app.manifest.packages.len()
                && app.installed_selected > 0
            {
                app.installed_selected -= 1;
            }
        }
    }
    let managed = write_manifest(app)?;
    app.log.push(format!(
        "Wrote {} ({}). {}...",
        managed.path().display(),
        app.config.target.label(),
        op.label(),
    ));
    Ok(())
}

fn move_selection(app: &mut App, delta: i32) {
    if app.results.is_empty() {
        return;
    }
    let len = app.results.len() as i32;
    let next = (app.selected as i32 + delta).rem_euclid(len);
    app.selected = next as usize;
}

fn move_installed_selection(app: &mut App, delta: i32) {
    let len = app.manifest.packages.len();
    if len == 0 {
        return;
    }
    let next = (app.installed_selected as i32 + delta).rem_euclid(len as i32);
    app.installed_selected = next as usize;
}

fn cycle_tab(app: &mut App) {
    let tabs = app.visible_tabs();
    let cur = tabs.iter().position(|t| *t == app.tab).unwrap_or(0);
    let next = tabs[(cur + 1) % tabs.len()];
    set_tab(app, next);
}

fn cycle_tab_back(app: &mut App) {
    let tabs = app.visible_tabs();
    let cur = tabs.iter().position(|t| *t == app.tab).unwrap_or(0);
    let prev = tabs[(cur + tabs.len() - 1) % tabs.len()];
    set_tab(app, prev);
}

fn set_tab(app: &mut App, tab: Tab) {
    app.tab = tab;
    if matches!(tab, Tab::Search) {
        app.search_input_mode = SearchInputMode::Normal;
    }
    app.status = format!("{} tab", tab.label());
}

fn toggle_target(app: &mut App) {
    app.config.target = match app.config.target {
        Target::HomeManager => Target::NixosSystem,
        Target::NixosSystem => Target::HomeManager,
    };
    let _ = app.config.save();
    app.status = format!("Target switched to {}", app.config.target.label());
}

fn cycle_theme(app: &mut App) {
    app.theme_cursor = app.theme_index;
    app.mode = Mode::ThemeSelect;
    app.status = "↑/↓ preview  ·  Enter confirm  ·  Esc cancel".into();
}

fn open_channel_edit(app: &mut App) {
    app.channel_cursor = CHANNELS
        .iter()
        .position(|c| *c == app.config.channel)
        .unwrap_or(0);
    app.mode = Mode::ChannelEdit;
    app.status = "j/k navigate  ↵ confirm  esc cancel".into();
}

fn write_manifest(app: &App) -> Result<ManagedFile> {
    let managed = ManagedFile::new(app.config.managed_file());
    match app.config.target {
        Target::HomeManager => managed.write_home_manager(&app.manifest)?,
        Target::NixosSystem => managed.write_nixos(&app.manifest)?,
    }
    Ok(managed)
}

fn spawn_rebuild(app: &mut App, tx: &mpsc::Sender<AppEvent>, action_label: String) {
    app.build_in_progress = true;
    app.current_op_label = Some(action_label.clone());
    app.log.clear();
    app.status = format!("{}...", action_label);

    let target = app.config.target;
    let config_dir = app.config.home_manager_dir();
    let app_tx = tx.clone();
    tokio::spawn(async move {
        let (build_tx, mut build_rx) = mpsc::channel::<BuildEvent>(64);
        let forward_tx = app_tx.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(ev) = build_rx.recv().await {
                if forward_tx.send(AppEvent::Build(ev)).await.is_err() {
                    break;
                }
            }
        });
        let cmd_owned;
        let args_owned;
        let (cmd, args): (&str, Vec<&str>) = match target {
            Target::HomeManager => {
                let (c, a) = home_manager_switch_cmd(&config_dir);
                cmd_owned = c;
                args_owned = a;
                (cmd_owned.as_str(), args_owned.iter().map(|s| s.as_str()).collect())
            }
            Target::NixosSystem => {
                let flake_ref = format!("{}#vm", config_dir.display());
                cmd_owned = "nix".into();
                args_owned = vec!["build".into(), flake_ref];
                (cmd_owned.as_str(), args_owned.iter().map(|s| s.as_str()).collect())
            }
        };
        if let Err(e) = rebuild(cmd, &args, build_tx.clone()).await {
            let _ = build_tx
                .send(BuildEvent::Finished(Err(e.to_string())))
                .await;
        }
        drop(build_tx);
        let _ = forwarder.await;
    });
}

async fn install_selected(app: &mut App, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
    let Some(hit) = app.results.get(app.selected).cloned() else {
        app.status = "No selection.".into();
        return Ok(());
    };

    if app.manifest.packages.contains(&hit.attr)
        || app.queue.iter().any(|op| matches!(op, QueuedOp::Install(h) if h.attr == hit.attr))
    {
        app.status = format!("{} already tracked or queued.", hit.attr);
        return Ok(());
    }

    let attr = hit.attr.clone();
    app.queue.push_back(QueuedOp::Install(hit));
    if app.build_in_progress {
        app.status = format!("Queued install: {}.", attr);
        app.tab = Tab::Queue;
    } else {
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

async fn uninstall_selected(app: &mut App, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
    let packages = app.installed_packages();
    let Some(pname) = packages.get(app.installed_selected).cloned() else {
        app.status = "No selection.".into();
        return Ok(());
    };

    if app.queue.iter().any(|op| matches!(op, QueuedOp::Uninstall(p) if *p == pname)) {
        app.status = format!("{} already queued for removal.", pname);
        return Ok(());
    }

    app.queue.push_back(QueuedOp::Uninstall(pname.clone()));
    if app.build_in_progress {
        app.status = format!("Queued remove: {}.", pname);
        app.tab = Tab::Queue;
    } else {
        app.tab = Tab::Building;
        drain_queue(app, tx);
    }
    Ok(())
}

fn schedule_search(app: &mut App, tx: mpsc::Sender<AppEvent>) {
    let query = app.input.value().to_string();
    if query.is_empty() {
        return;
    }
    app.searching = true;
    app.search_epoch += 1;
    let epoch = app.search_epoch;
    let channel = app.config.channel.clone();
    app.latest_query = query.clone();

    tokio::spawn(async move {
        sleep(Duration::from_millis(180)).await;
        match search(&channel, &query).await {
            Ok(hits) => {
                let _ = tx.send(AppEvent::SearchDone { epoch, hits }).await;
            }
            Err(e) => {
                let _ = tx
                    .send(AppEvent::SearchFailed {
                        epoch,
                        error: e.to_string(),
                    })
                    .await;
            }
        }
    });
}

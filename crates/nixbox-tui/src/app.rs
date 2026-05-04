use std::io;
use std::time::Duration;

use anyhow::Result;
use nixbox_config::{Config, Target};
use nixbox_nix::{
    build::{home_manager_switch_cmd, rebuild, BuildEvent},
    manifest::{ensure_home_nix, ManagedFile},
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

use crate::ui;

#[derive(Debug, Clone)]
pub(crate) enum Mode {
    Search,
    Building,
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
    pub(crate) status: String,
    pub(crate) mode: Mode,
    pub(crate) log: Vec<String>,
    pub(crate) search_epoch: u64,
    pub(crate) latest_query: String,
    pub(crate) should_quit: bool,
}

impl App {
    fn new(config: Config, manifest: Manifest) -> Self {
        let installed = manifest.packages.len();
        Self {
            config,
            manifest,
            input: Input::default(),
            results: Vec::new(),
            selected: 0,
            status: format!("{} packages tracked. Type to search.", installed),
            mode: Mode::Search,
            log: Vec::new(),
            search_epoch: 0,
            latest_query: String::new(),
            should_quit: false,
        }
    }

    pub(crate) fn channel(&self) -> &str {
        &self.config.channel
    }

    pub(crate) fn target_label(&self) -> &'static str {
        self.config.target.label()
    }
}

pub async fn run() -> Result<()> {
    let config = Config::load_or_default()?;
    let managed = ManagedFile::new(config.managed_file.clone());
    let manifest = managed.load()?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, config, manifest, managed).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: Config,
    manifest: Manifest,
    managed: ManagedFile,
) -> Result<()> {
    let mut app = App::new(config, manifest);
    let (tx, mut rx) = mpsc::channel::<AppEvent>(128);
    let mut term_events = EventStream::new();

    schedule_search(&mut app, tx.clone());

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;
        if app.should_quit {
            break;
        }

        tokio::select! {
            Some(Ok(ev)) = term_events.next() => {
                handle_terminal_event(&mut app, &managed, &tx, ev).await?;
            }
            Some(app_ev) = rx.recv() => {
                handle_app_event(&mut app, app_ev);
            }
        }
    }
    Ok(())
}

async fn handle_terminal_event(
    app: &mut App,
    managed: &ManagedFile,
    tx: &mpsc::Sender<AppEvent>,
    ev: CtEvent,
) -> Result<()> {
    match (&app.mode, ev) {
        (Mode::Building, CtEvent::Key(key)) if key.kind == KeyEventKind::Press => {
            if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                app.mode = Mode::Search;
                app.status = "Returned to search.".into();
            }
        }
        (Mode::Search, CtEvent::Key(key)) if key.kind == KeyEventKind::Press => {
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c'))
            {
                app.should_quit = true;
                return Ok(());
            }
            match key.code {
                KeyCode::Esc => app.should_quit = true,
                KeyCode::Down => move_selection(app, 1),
                KeyCode::Up => move_selection(app, -1),
                KeyCode::Tab => toggle_target(app),
                KeyCode::Enter => install_selected(app, managed, tx).await?,
                _ => {
                    let before = app.input.value().to_string();
                    app.input.handle_event(&CtEvent::Key(key));
                    if app.input.value() != before {
                        schedule_search(app, tx.clone());
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_app_event(app: &mut App, ev: AppEvent) {
    match ev {
        AppEvent::SearchDone { epoch, hits } => {
            if epoch == app.search_epoch {
                let count = hits.len();
                app.results = hits;
                app.selected = 0;
                app.status = format!("{} matches for `{}`", count, app.latest_query);
            }
        }
        AppEvent::SearchFailed { epoch, error } => {
            if epoch == app.search_epoch {
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
        AppEvent::Build(BuildEvent::Finished(Ok(()))) => {
            app.status = "Build succeeded. Press q to return.".into();
        }
        AppEvent::Build(BuildEvent::Finished(Err(err))) => {
            app.status = format!("Build failed: {}. Press q to return.", err);
        }
    }
}

fn move_selection(app: &mut App, delta: i32) {
    if app.results.is_empty() {
        return;
    }
    let len = app.results.len() as i32;
    let next = (app.selected as i32 + delta).rem_euclid(len);
    app.selected = next as usize;
}

fn toggle_target(app: &mut App) {
    app.config.target = match app.config.target {
        Target::HomeManager => Target::NixosSystem,
        Target::NixosSystem => Target::HomeManager,
    };
    let _ = app.config.save();
    app.status = format!("Target switched to {}", app.config.target.label());
}

async fn install_selected(
    app: &mut App,
    managed: &ManagedFile,
    tx: &mpsc::Sender<AppEvent>,
) -> Result<()> {
    let Some(hit) = app.results.get(app.selected).cloned() else {
        app.status = "No selection.".into();
        return Ok(());
    };

    let added = app.manifest.add(&hit.pname);
    if !added {
        app.status = format!("{} already tracked.", hit.pname);
        return Ok(());
    }

    match app.config.target {
        Target::HomeManager => managed.write_home_manager(&app.manifest)?,
        Target::NixosSystem => managed.write_nixos(&app.manifest)?,
    }

    app.mode = Mode::Building;
    app.log.clear();
    app.log.push(format!(
        "Wrote {} ({}). Starting rebuild...",
        managed.path().display(),
        app.config.target.label()
    ));
    app.status = format!("Building {}...", hit.pname);

    let target = app.config.target;
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
        let hm_cmd;
        let hm_args_owned;
        let (cmd, args): (&str, Vec<&str>) = match target {
            Target::HomeManager => {
                let (c, a) = home_manager_switch_cmd();
                hm_cmd = c;
                hm_args_owned = a;
                (hm_cmd.as_str(), hm_args_owned.iter().map(|s| s.as_str()).collect())
            }
            Target::NixosSystem => ("sudo", vec!["nixos-rebuild", "switch"]),
        };
        if let Err(e) = rebuild(cmd, &args, build_tx.clone()).await {
            let _ = build_tx
                .send(BuildEvent::Finished(Err(e.to_string())))
                .await;
        }
        drop(build_tx);
        let _ = forwarder.await;
    });

    Ok(())
}

fn schedule_search(app: &mut App, tx: mpsc::Sender<AppEvent>) {
    app.search_epoch += 1;
    let epoch = app.search_epoch;
    let query = app.input.value().to_string();
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

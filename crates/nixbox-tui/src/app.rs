use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use anyhow::Result;
use nixbox_config::{Config, Target};
use nixbox_nix::{
    build::BuildEvent,
    manifest::ManagedFile,
    scan::{scan, ExternalPackage, ScanTarget},
    search::SearchHit,
    Manifest,
};
use crossterm::event::EventStream;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;
use tui_input::Input;

use crate::handlers::{handle_app_event, handle_terminal_event};
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
    Install { hit: SearchHit, scope: Target },
    Uninstall { name: String, scope: Target },
    Migrate { names: Vec<String>, scope: Target },
}

impl QueuedOp {
    pub(crate) fn scope(&self) -> Target {
        match self {
            QueuedOp::Install { scope, .. }
            | QueuedOp::Uninstall { scope, .. }
            | QueuedOp::Migrate { scope, .. } => *scope,
        }
    }

    pub(crate) fn label(&self) -> String {
        let tag = self.scope().tag();
        match self {
            QueuedOp::Install { hit, .. } => format!("install {} [{}]", hit.attr, tag),
            QueuedOp::Uninstall { name, .. } => format!("remove {} [{}]", name, tag),
            QueuedOp::Migrate { names, .. } => match names.len() {
                1 => format!("migrate {} [{}]", names[0], tag),
                n => format!("migrate {} packages [{}]", n, tag),
            },
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

/// A package tracked by nixbox's manifest (managed) — tagged with the
/// target/scope it belongs to.
#[derive(Debug, Clone)]
pub(crate) struct ManagedPackage {
    pub name: String,
    pub scope: Target,
}

/// The row currently under the cursor in the Installed tab.
#[derive(Debug, Clone)]
pub(crate) enum InstalledCursor {
    Managed(ManagedPackage),
    External(ExternalPackage),
}

pub(crate) struct App {
    pub(crate) config: Config,
    /// Manifest of packages tracked in `nixbox-home-packages.nix`.
    pub(crate) home_manifest: Manifest,
    /// Manifest of packages tracked in `nixbox-system-packages.nix`.
    pub(crate) nixos_manifest: Manifest,
    /// Packages found declared directly in the user's main config files
    /// (both home.nix and configuration.nix), tagged with their scope.
    pub(crate) external_packages: Vec<ExternalPackage>,
    pub(crate) input: Input,
    pub(crate) results: Vec<SearchHit>,
    pub(crate) selected: usize,
    pub(crate) installed_input: Input,
    pub(crate) installed_input_mode: SearchInputMode,
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
    fn new(
        config: Config,
        home_manifest: Manifest,
        nixos_manifest: Manifest,
        external_packages: Vec<ExternalPackage>,
    ) -> Self {
        let managed = home_manifest.packages.len() + nixos_manifest.packages.len();
        let external = external_packages.len();
        let theme_index = theme::ALL
            .iter()
            .position(|t| t.name == config.theme)
            .unwrap_or(0);
        let status = if external == 0 {
            format!("{} packages tracked.", managed)
        } else {
            format!("{} managed  ·  {} external.", managed, external)
        };
        Self {
            config,
            home_manifest,
            nixos_manifest,
            external_packages,
            input: Input::default(),
            results: Vec::new(),
            selected: 0,
            installed_input: Input::default(),
            installed_input_mode: SearchInputMode::Normal,
            installed_selected: 0,
            status,
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
        if self.build_in_progress || !self.log.is_empty() {
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

    pub(crate) fn manifest_for(&self, scope: Target) -> &Manifest {
        match scope {
            Target::HomeManager => &self.home_manifest,
            Target::NixosSystem => &self.nixos_manifest,
        }
    }

    pub(crate) fn manifest_for_mut(&mut self, scope: Target) -> &mut Manifest {
        match scope {
            Target::HomeManager => &mut self.home_manifest,
            Target::NixosSystem => &mut self.nixos_manifest,
        }
    }

    /// Returns all managed packages from both scopes, sorted by (scope, name)
    /// so HM entries appear before NixOS entries.
    pub(crate) fn managed_packages(&self) -> Vec<ManagedPackage> {
        let mut out: Vec<ManagedPackage> = Vec::new();
        for p in &self.home_manifest.packages {
            out.push(ManagedPackage {
                name: p.clone(),
                scope: Target::HomeManager,
            });
        }
        for p in &self.nixos_manifest.packages {
            out.push(ManagedPackage {
                name: p.clone(),
                scope: Target::NixosSystem,
            });
        }
        out
    }

    pub(crate) fn installed_filter(&self) -> Option<String> {
        let q = self.installed_input.value().trim().to_lowercase();
        if q.is_empty() { None } else { Some(q) }
    }

    pub(crate) fn filtered_managed_packages(&self) -> Vec<ManagedPackage> {
        let filter = self.installed_filter();
        self.managed_packages()
            .into_iter()
            .filter(|p| match &filter {
                None => true,
                Some(q) => p.name.to_lowercase().contains(q),
            })
            .collect()
    }

    pub(crate) fn filtered_external_packages(&self) -> Vec<ExternalPackage> {
        let filter = self.installed_filter();
        self.external_packages
            .iter()
            .filter(|ep| match &filter {
                None => true,
                Some(q) => ep.name.to_lowercase().contains(q),
            })
            .cloned()
            .collect()
    }

    pub(crate) fn installed_total(&self) -> usize {
        self.filtered_managed_packages().len() + self.filtered_external_packages().len()
    }

    /// Returns the row currently under the cursor in the Installed tab.
    pub(crate) fn installed_cursor(&self) -> Option<InstalledCursor> {
        let managed = self.filtered_managed_packages();
        let external = self.filtered_external_packages();
        let total = managed.len() + external.len();
        if total == 0 {
            return None;
        }
        let idx = self.installed_selected.min(total - 1);
        if idx < managed.len() {
            Some(InstalledCursor::Managed(managed[idx].clone()))
        } else {
            Some(InstalledCursor::External(
                external[idx - managed.len()].clone(),
            ))
        }
    }

    /// Re-reads both main config files and refreshes `external_packages`,
    /// excluding anything already tracked in either manifest.
    pub(crate) fn refresh_external_packages(&mut self) {
        self.external_packages = read_external_packages(
            &self.config,
            &self.home_manifest,
            &self.nixos_manifest,
        );
        let total = self.installed_total();
        if total == 0 {
            self.installed_selected = 0;
        } else if self.installed_selected >= total {
            self.installed_selected = total - 1;
        }
    }
}

/// Scans both home.nix and configuration.nix and returns external packages
/// from each, scope-tagged, excluding anything already in the matching
/// manifest. The same package name can appear twice if declared in both
/// scopes — that's intentional.
pub(crate) fn read_external_packages(
    config: &Config,
    home_manifest: &Manifest,
    nixos_manifest: &Manifest,
) -> Vec<ExternalPackage> {
    let mut out: Vec<ExternalPackage> = Vec::new();

    if let Ok(found) = scan(
        &config.main_file_for(Target::HomeManager),
        ScanTarget::HomeManager,
    ) {
        out.extend(
            found
                .into_iter()
                .filter(|ep| !home_manifest.packages.contains(&ep.name)),
        );
    }
    if let Ok(found) = scan(
        &config.main_file_for(Target::NixosSystem),
        ScanTarget::Nixos,
    ) {
        out.extend(
            found
                .into_iter()
                .filter(|ep| !nixos_manifest.packages.contains(&ep.name)),
        );
    }

    out
}

pub async fn run() -> Result<()> {
    let config = Config::load_or_default()?;
    let home_manifest = ManagedFile::new(config.managed_file_for(Target::HomeManager)).load()?;
    let nixos_manifest = ManagedFile::new(config.managed_file_for(Target::NixosSystem)).load()?;
    let externals = read_external_packages(&config, &home_manifest, &nixos_manifest);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(
        &mut terminal,
        config,
        home_manifest,
        nixos_manifest,
        externals,
    )
    .await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: Config,
    home_manifest: Manifest,
    nixos_manifest: Manifest,
    externals: Vec<ExternalPackage>,
) -> Result<()> {
    let mut app = App::new(config, home_manifest, nixos_manifest, externals);
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

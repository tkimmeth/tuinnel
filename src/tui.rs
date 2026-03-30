// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// tui.rs — Full-screen dashboard TUI with spinning globe + overlay system
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Layout:
//   ┌─ tuinnel ────────────────────────────────────────────────────────┐
//   │  [Globe]          │  [Connection Info]                           │
//   │  Spinning braille │  Server, City, IP, Protocol, Uptime, KS     │
//   │  globe with       ├─────────────────────────────────────────────│
//   │  server marker    │  [Bandwidth]                                 │
//   │                   │  RX/TX sparklines                            │
//   ├───────────────────┼─────────────────────────────────────────────│
//   │  [Actions]        │  [Network]                                   │
//   │  > Reconnect      │  SSID, Route, DNS, Leak                     │
//   │    Change Server  │                                              │
//   │    Disconnect     │                                              │
//   └─ j/k  Enter  r  q ──────────────────── tuinnel v0.2.0 ─────────┘

use crate::backend::{ConnectTarget, SessionInfo, VpnBackend, VpnManager};
use crate::bandwidth::BandwidthMonitor;
use crate::config::Config;
use crate::geo;
use crate::globe::{self, WorldMap};
use crate::net;
use crate::output::OutputBuffer;
use crate::security;
use crate::servers::{self, ServerEntry};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;

use ratatui::prelude::*;
use ratatui::symbols::Marker;
use ratatui::widgets::canvas::{Canvas, Points};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Sparkline};

use std::io::{self, stdout, IsTerminal};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

// ── Constants ────────────────────────────────────────────────────────────────

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

// ── Menu Items ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum MenuAction {
    Reconnect,
    ChangeServer,
    Disconnect,
    KillSwitch,
    Refresh,
    SecurityAudit,
    Doctor,
    Quit,
}

const MENU_ACTIONS: &[(MenuAction, &str)] = &[
    (MenuAction::Reconnect, "Reconnect"),
    (MenuAction::ChangeServer, "Change Server"),
    (MenuAction::Disconnect, "Disconnect"),
    (MenuAction::KillSwitch, "Kill Switch"),
    (MenuAction::Refresh, "Refresh"),
    (MenuAction::SecurityAudit, "Security Audit"),
    (MenuAction::Doctor, "Doctor"),
    (MenuAction::Quit, "Quit"),
];

// ── Background Messages ─────────────────────────────────────────────────────

enum AppMessage {
    ActionDone {
        title: String,
        buf: OutputBuffer,
    },
    ConnectResult {
        result: Result<SessionInfo, String>,
    },
    DisconnectResult {
        result: Result<(), String>,
    },
    StatsRefreshed {
        ip: String,
        net_info: NetworkInfo,
    },
}

// ── Server Picker ──────────────────────────────────────────────────────────

#[derive(Clone)]
struct PickerEntry {
    display: String,
    value: String,
}

enum PickerStep {
    Countries,
    Cities { country_code: String, country_name: String },
}

struct PickerState {
    step: PickerStep,
    all_entries: Vec<PickerEntry>,
    filtered: Vec<usize>,
    query: String,
    selected: usize,
    scroll_offset: usize,
}

impl PickerState {
    fn new(step: PickerStep, entries: Vec<PickerEntry>) -> Self {
        let filtered: Vec<usize> = (0..entries.len()).collect();
        Self {
            step,
            all_entries: entries,
            filtered,
            query: String::new(),
            selected: 0,
            scroll_offset: 0,
        }
    }

    fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        if q.is_empty() {
            self.filtered = (0..self.all_entries.len()).collect();
        } else {
            self.filtered = self.all_entries.iter().enumerate()
                .filter(|(_, e)| fuzzy_match(&e.display.to_lowercase(), &q))
                .map(|(i, _)| i)
                .collect();
        }
        self.selected = 0;
        self.scroll_offset = 0;
    }

    fn selected_entry(&self) -> Option<&PickerEntry> {
        self.filtered.get(self.selected).map(|&i| &self.all_entries[i])
    }
}

fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    let mut hay = haystack.chars().peekable();
    for nc in needle.chars() {
        loop {
            match hay.next() {
                Some(hc) if hc == nc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

// ── Overlay State Machine ───────────────────────────────────────────────────

struct Overlay {
    title: String,
    lines: Vec<Line<'static>>,
    scroll: usize,
}

enum OverlayState {
    None,
    Running {
        title: String,
        spinner_frame: usize,
        started: Instant,
    },
    Done(Overlay),
    Picker(PickerState),
}

// ── App State ───────────────────────────────────────────────────────────────

struct ConnectionInfo {
    connected: bool,
    server: String,
    city: String,
    ip: String,
    protocol: String,
    kill_switch: String,
}

struct NetworkInfo {
    ssid: String,
    route: String,
    dns: String,
    dns_safe: bool,
    leak: String,
}

struct App {
    // Menu
    list_state: ListState,
    should_quit: bool,

    // Globe
    globe_rotation: f64,
    world_map: WorldMap,
    server_coords: Option<(f64, f64)>,

    // Connection
    conn_info: ConnectionInfo,
    net_info: NetworkInfo,
    connect_time: Option<Instant>,

    // Bandwidth
    bandwidth: BandwidthMonitor,

    // Refresh timers
    last_stats_refresh: Instant,
    last_bw_sample: Instant,

    // Overlay state machine
    overlay: OverlayState,

    // Shared state
    backend: Arc<dyn VpnBackend>,
    servers: Vec<ServerEntry>,
    session: Option<SessionInfo>,
    config: Arc<Config>,
    msg_tx: mpsc::Sender<AppMessage>,
    msg_rx: mpsc::Receiver<AppMessage>,
    stats_refresh_pending: bool,
}

impl App {
    fn new(
        backend: Arc<dyn VpnBackend>,
        servers: Vec<ServerEntry>,
        session: Option<SessionInfo>,
        config: Arc<Config>,
    ) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        let world_map = WorldMap::generate();
        let conn_info = Self::connection_info_from_session(&session);

        let mut bandwidth = BandwidthMonitor::new();
        if let Some(ref s) = session {
            bandwidth.set_interface(Some(s.interface.clone()));
        }

        let server_coords = session.as_ref().and_then(|s| {
            match (s.lat, s.lon) {
                (Some(lat), Some(lon)) => Some((lat, lon)),
                _ => geo::lookup_city_name(&s.city),
            }
        });

        let connect_time = if session.is_some() {
            Some(Instant::now())
        } else {
            None
        };

        let net_info = Self::gather_network_info(session.as_ref());

        let (msg_tx, msg_rx) = mpsc::channel();

        Self {
            list_state,
            should_quit: false,
            globe_rotation: 0.0,
            world_map,
            server_coords,
            conn_info,
            net_info,
            connect_time,
            bandwidth,
            last_stats_refresh: Instant::now(),
            last_bw_sample: Instant::now(),
            overlay: OverlayState::None,
            backend,
            servers,
            session,
            config,
            msg_tx,
            msg_rx,
            stats_refresh_pending: false,
        }
    }

    fn selected(&self) -> usize {
        self.list_state.selected().unwrap_or(0)
    }

    fn selected_action(&self) -> MenuAction {
        MENU_ACTIONS[self.selected()].0
    }

    fn next(&mut self) {
        let i = (self.selected() + 1) % MENU_ACTIONS.len();
        self.list_state.select(Some(i));
    }

    fn prev(&mut self) {
        let i = if self.selected() == 0 {
            MENU_ACTIONS.len() - 1
        } else {
            self.selected() - 1
        };
        self.list_state.select(Some(i));
    }

    fn connection_info_from_session(session: &Option<SessionInfo>) -> ConnectionInfo {
        match session {
            Some(s) => {
                let server = s.display_name.clone();
                let city = if s.city.is_empty() { "--".into() } else { s.city.clone() };
                let protocol = s.protocol.to_string();
                let kill_switch = if security::check_kill_switch_firewall().passed {
                    "ON (firewall rules active)".into()
                } else {
                    "OFF".into()
                };

                ConnectionInfo {
                    connected: true,
                    server,
                    city,
                    ip: "--".into(), // filled by stats refresh
                    protocol,
                    kill_switch,
                }
            }
            None => ConnectionInfo {
                connected: false,
                server: "--".into(),
                city: "--".into(),
                ip: "--".into(),
                protocol: "--".into(),
                kill_switch: "OFF".into(),
            },
        }
    }

    fn gather_network_info(session: Option<&SessionInfo>) -> NetworkInfo {
        let ssid = net::wifi_ssid();

        let route_raw = net::default_route();
        let route = route_raw.lines().next().unwrap_or("--").to_string();

        let dns_raw = net::dns_servers();
        let dns_line = dns_raw
            .lines()
            .filter(|l| l.contains("nameserver") || l.contains("10.") || l.contains("DNS"))
            .map(|l| l.trim())
            .next()
            .unwrap_or_else(|| dns_raw.lines().next().unwrap_or("--"))
            .to_string();

        let expected_dns = session
            .map(|s| s.dns_servers.as_slice())
            .unwrap_or(&[]);
        let dns_check = security::check_dns_leak(expected_dns);
        let dns_safe = dns_check.passed;

        let dns = if dns_safe && !expected_dns.is_empty() {
            format!("{dns_line} (VPN DNS)")
        } else {
            dns_line
        };

        let iface = session.map(|s| s.interface.as_str());
        let (tunneled, _) = net::is_tunneled(iface);
        let ipv6_check = security::check_ipv6_leak(session);

        let leak = if tunneled && dns_safe && ipv6_check.passed {
            "None detected".to_string()
        } else {
            let mut issues = Vec::new();
            if !tunneled { issues.push("traffic not tunneled"); }
            if !dns_safe { issues.push("DNS leak"); }
            if !ipv6_check.passed { issues.push("IPv6 leak"); }
            if issues.is_empty() {
                "None detected".to_string()
            } else {
                format!("WARNING: {}", issues.join(", "))
            }
        };

        NetworkInfo { ssid, route, dns, dns_safe, leak }
    }

    fn spawn_stats_refresh(&mut self) {
        if self.stats_refresh_pending {
            return;
        }
        self.stats_refresh_pending = true;
        let session = self.session.clone();
        let tx = self.msg_tx.clone();
        thread::spawn(move || {
            let ip = net::public_ip();
            let net_info = App::gather_network_info(session.as_ref());
            let _ = tx.send(AppMessage::StatsRefreshed { ip, net_info });
        });
    }

    fn apply_stats_refresh(&mut self, ip: String, net_info: NetworkInfo) {
        self.conn_info = Self::connection_info_from_session(&self.session);
        self.conn_info.ip = ip;
        self.net_info = net_info;

        if let Some(ref s) = self.session {
            self.bandwidth.set_interface(Some(s.interface.clone()));
            self.server_coords = match (s.lat, s.lon) {
                (Some(lat), Some(lon)) => Some((lat, lon)),
                _ => geo::lookup_city_name(&s.city),
            };
        } else {
            self.bandwidth.set_interface(None);
            self.server_coords = None;
        }

        if self.conn_info.connected && self.connect_time.is_none() {
            self.connect_time = Some(Instant::now());
        } else if !self.conn_info.connected {
            self.connect_time = None;
        }

        self.stats_refresh_pending = false;
        self.last_stats_refresh = Instant::now();
    }

    fn uptime_string(&self) -> String {
        match self.connect_time {
            Some(t) => {
                let secs = t.elapsed().as_secs();
                if secs >= 3600 {
                    format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
                } else if secs >= 60 {
                    format!("{}m {:02}s", secs / 60, secs % 60)
                } else {
                    format!("{}s", secs)
                }
            }
            None => "--".to_string(),
        }
    }
}

// ── Entry Point ─────────────────────────────────────────────────────────────

pub fn run(config: Arc<Config>, manager: &mut VpnManager) -> anyhow::Result<()> {
    if !io::stdout().is_terminal() {
        anyhow::bail!("TUI requires a terminal (TTY)");
    }

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    let ratatui_backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(ratatui_backend)?;

    let mut app = App::new(
        Arc::clone(&manager.backend),
        manager.servers.clone(),
        manager.session.clone(),
        config,
    );

    let result = main_loop(&mut terminal, &mut app);

    // Sync session back to manager
    manager.session = app.session;

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

// ── Main Loop ───────────────────────────────────────────────────────────────

fn main_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|frame| render(frame, app))?;

        if crossterm::event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if !matches!(app.overlay, OverlayState::None) {
                    handle_overlay_key(app, key.code);
                } else {
                    handle_dashboard_key(app, key.code)?;
                }
            }
        }

        // Process background messages
        while let Ok(msg) = app.msg_rx.try_recv() {
            match msg {
                AppMessage::ActionDone { title, buf } => {
                    if matches!(app.overlay, OverlayState::Running { .. }) {
                        app.overlay = OverlayState::Done(Overlay {
                            title,
                            lines: buf.to_ratatui_lines(),
                            scroll: 0,
                        });
                    }
                    app.spawn_stats_refresh();
                }
                AppMessage::ConnectResult { result } => {
                    match result {
                        Ok(session) => {
                            let mut buf = OutputBuffer::new();
                            buf.ok("Connected!");
                            buf.kv("Server", &session.display_name);
                            buf.kv("Interface", &session.interface);
                            buf.kv("Protocol", &session.protocol.to_string());
                            app.session = Some(session);
                            app.connect_time = Some(Instant::now());
                            if matches!(app.overlay, OverlayState::Running { .. }) {
                                app.overlay = OverlayState::Done(Overlay {
                                    title: "Connect".into(),
                                    lines: buf.to_ratatui_lines(),
                                    scroll: 0,
                                });
                            }
                        }
                        Err(e) => {
                            let mut buf = OutputBuffer::new();
                            buf.err(&format!("Connection failed: {e}"));
                            if matches!(app.overlay, OverlayState::Running { .. }) {
                                app.overlay = OverlayState::Done(Overlay {
                                    title: "Connect".into(),
                                    lines: buf.to_ratatui_lines(),
                                    scroll: 0,
                                });
                            }
                        }
                    }
                    app.spawn_stats_refresh();
                }
                AppMessage::DisconnectResult { result } => {
                    let mut buf = OutputBuffer::new();
                    match result {
                        Ok(()) => {
                            buf.ok("Disconnected.");
                            app.session = None;
                            app.connect_time = None;
                        }
                        Err(e) => {
                            buf.err(&format!("Disconnect failed: {e}"));
                        }
                    }
                    if matches!(app.overlay, OverlayState::Running { .. }) {
                        app.overlay = OverlayState::Done(Overlay {
                            title: "Disconnect".into(),
                            lines: buf.to_ratatui_lines(),
                            scroll: 0,
                        });
                    }
                    app.spawn_stats_refresh();
                }
                AppMessage::StatsRefreshed { ip, net_info } => {
                    app.apply_stats_refresh(ip, net_info);
                }
            }
        }

        // Advance spinner frame when running
        if let OverlayState::Running { ref mut spinner_frame, .. } = app.overlay {
            *spinner_frame += 1;
        }

        // Advance globe rotation
        app.globe_rotation += 1.0;
        if app.globe_rotation >= 360.0 {
            app.globe_rotation -= 360.0;
        }

        // Sample bandwidth every second
        if app.last_bw_sample.elapsed() >= Duration::from_secs(1) {
            app.bandwidth.sample();
            app.last_bw_sample = Instant::now();
        }

        // Background stats refresh every 30 seconds
        if app.last_stats_refresh.elapsed() >= Duration::from_secs(30) {
            app.spawn_stats_refresh();
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

/// Handle key presses when an overlay is visible.
fn handle_overlay_key(app: &mut App, code: KeyCode) {
    if matches!(app.overlay, OverlayState::Picker(_)) {
        handle_picker_key(app, code);
        return;
    }

    match app.overlay {
        OverlayState::Running { .. } => {
            if matches!(code, KeyCode::Esc) {
                app.overlay = OverlayState::None;
            }
        }
        OverlayState::Done(ref mut ov) => match code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                app.overlay = OverlayState::None;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if ov.scroll + 1 < ov.lines.len() {
                    ov.scroll += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                ov.scroll = ov.scroll.saturating_sub(1);
            }
            _ => {}
        },
        _ => {}
    }
}

fn handle_picker_key(app: &mut App, code: KeyCode) {
    let picker = match &mut app.overlay {
        OverlayState::Picker(p) => p,
        _ => return,
    };

    match code {
        KeyCode::Down => {
            if picker.selected + 1 < picker.filtered.len() {
                picker.selected += 1;
            }
        }
        KeyCode::Up => {
            picker.selected = picker.selected.saturating_sub(1);
        }
        KeyCode::Backspace => {
            if !picker.query.is_empty() {
                picker.query.pop();
                picker.refilter();
            }
        }
        KeyCode::Enter => {
            let entry = match picker.selected_entry() {
                Some(e) => e.clone(),
                None => return,
            };
            match &picker.step {
                PickerStep::Countries => {
                    let cc = entry.value.clone();
                    let name = entry.display.clone();
                    // Instantly get cities from local server list
                    let city_list = servers::cities(&app.servers, &cc);
                    if city_list.is_empty() {
                        // No cities — connect directly to country
                        app.overlay = OverlayState::Running {
                            title: format!("Connecting to {name}"),
                            spinner_frame: 0,
                            started: Instant::now(),
                        };
                        let matches = servers::find_by_country(&app.servers, &cc);
                        if let Some(server) = matches.first() {
                            let server = (*server).clone();
                            let backend = Arc::clone(&app.backend);
                            let tx = app.msg_tx.clone();
                            thread::spawn(move || {
                                let result = backend.connect(&server);
                                let _ = tx.send(AppMessage::ConnectResult { result });
                            });
                        }
                    } else {
                        let entries: Vec<PickerEntry> = city_list
                            .into_iter()
                            .map(|c| PickerEntry { display: c.clone(), value: c })
                            .collect();
                        app.overlay = OverlayState::Picker(PickerState::new(
                            PickerStep::Cities { country_code: cc, country_name: name },
                            entries,
                        ));
                    }
                }
                PickerStep::Cities { .. } => {
                    let city = entry.value.clone();
                    app.overlay = OverlayState::Running {
                        title: format!("Connecting to {city}"),
                        spinner_frame: 0,
                        started: Instant::now(),
                    };
                    let matches = servers::find_by_city(&app.servers, &city);
                    if let Some(server) = matches.first() {
                        let server = (*server).clone();
                        let backend = Arc::clone(&app.backend);
                        let tx = app.msg_tx.clone();
                        thread::spawn(move || {
                            let result = backend.connect(&server);
                            let _ = tx.send(AppMessage::ConnectResult { result });
                        });
                    }
                }
            }
        }
        KeyCode::Esc => {
            let go_back_to_countries = matches!(picker.step, PickerStep::Cities { .. });
            if go_back_to_countries {
                // Rebuild country picker from local data
                let country_entries: Vec<PickerEntry> = servers::countries(&app.servers)
                    .into_iter()
                    .map(|(code, _)| {
                        let count = servers::find_by_country(&app.servers, &code).len();
                        PickerEntry {
                            display: format!("{code}  ({count} server(s))"),
                            value: code,
                        }
                    })
                    .collect();
                app.overlay = OverlayState::Picker(PickerState::new(
                    PickerStep::Countries, country_entries,
                ));
            } else {
                app.overlay = OverlayState::None;
            }
        }
        KeyCode::Char(c) => {
            picker.query.push(c);
            picker.refilter();
        }
        _ => {}
    }
}

/// Handle key presses on the main dashboard.
fn handle_dashboard_key(app: &mut App, code: KeyCode) -> anyhow::Result<()> {
    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            app.should_quit = true;
        }
        KeyCode::Up | KeyCode::Char('k') => app.prev(),
        KeyCode::Down | KeyCode::Char('j') => app.next(),
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.spawn_stats_refresh();
        }
        KeyCode::Enter => {
            handle_selection(app);
        }
        _ => {}
    }
    Ok(())
}

// ── Rendering ───────────────────────────────────────────────────────────────

fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let outer = Block::default()
        .borders(Borders::ALL)
        .title(" tuinnel ")
        .title_alignment(Alignment::Left)
        .border_style(Style::default().fg(Color::Cyan))
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(55),
            Constraint::Percentage(40),
            Constraint::Length(1),
        ])
        .split(inner);

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(main_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(top_chunks[1]);

    let bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(main_chunks[1]);

    render_globe(frame, app, top_chunks[0]);
    render_connection(frame, app, right_chunks[0]);
    render_bandwidth(frame, app, right_chunks[1]);
    render_actions(frame, app, bottom_chunks[0]);
    render_network(frame, app, bottom_chunks[1]);
    render_footer(frame, main_chunks[2]);

    match &app.overlay {
        OverlayState::None => {}
        OverlayState::Running { title, spinner_frame, started } => {
            render_spinner_overlay(frame, title, *spinner_frame, *started, area);
        }
        OverlayState::Done(overlay) => {
            render_result_overlay(frame, overlay, area);
        }
        OverlayState::Picker(picker) => {
            render_picker_overlay(frame, picker, area);
        }
    }
}

fn render_globe(frame: &mut Frame, app: &App, area: Rect) {
    let land_points = globe::project_land(&app.world_map, app.globe_rotation);
    let outline = globe::globe_outline();

    let land_coords: Vec<(f64, f64)> = land_points.iter().map(|p| (p.x, p.y)).collect();

    let user_coords = app.config.general.user_lat
        .zip(app.config.general.user_lon);

    let user_point = user_coords.and_then(|(lat, lon)| {
        globe::project_point(lat, lon, app.globe_rotation)
    });

    let server_point = app.server_coords.and_then(|(lat, lon)| {
        globe::project_point(lat, lon, app.globe_rotation)
    });

    let arc_origin = user_coords.unwrap_or((geo::FALLBACK_USER_LAT, geo::FALLBACK_USER_LON));
    let arc_points = app.server_coords.map(|(slat, slon)| {
        globe::great_circle_arc(
            arc_origin.0,
            arc_origin.1,
            slat,
            slon,
            app.globe_rotation,
            200,
        )
    });

    let canvas = Canvas::default()
        .block(Block::default())
        .marker(Marker::Braille)
        .x_bounds([-1.3, 1.3])
        .y_bounds([-1.3, 1.3])
        .paint(move |ctx| {
            let outline_coords: Vec<(f64, f64)> = outline.clone();
            ctx.draw(&Points {
                coords: &outline_coords,
                color: Color::DarkGray,
            });

            ctx.draw(&Points {
                coords: &land_coords,
                color: Color::Green,
            });

            let tick = app.globe_rotation as usize;

            if let Some(ref arc) = arc_points {
                if arc.len() >= 2 {
                    ctx.draw(&Points {
                        coords: arc,
                        color: Color::Rgb(0, 130, 60),
                    });
                    let total = arc.len();
                    let step = (total / 8).max(1);
                    for (i, &(x, y)) in arc.iter().enumerate() {
                        if i == 0 || i == total - 1 { continue; }
                        if (i + tick * 3) % step == 0 {
                            ctx.print(x, y,
                                Span::styled("•", Style::default().fg(Color::Rgb(0, 255, 100))),
                            );
                        }
                    }
                }
            }

            if let Some((ux, uy)) = user_point {
                let ch = if tick / 6 % 2 == 0 { '◆' } else { '◇' };
                ctx.print(ux, uy,
                    Span::styled(String::from(ch), Style::default().fg(Color::Rgb(0, 255, 100))),
                );
            }

            if let Some((sx, sy)) = server_point {
                let bright = ((tick as f64 / 4.0).sin() * 60.0 + 195.0) as u8;
                let ch = if tick / 4 % 2 == 0 { '⊕' } else { '⊙' };
                ctx.print(sx, sy,
                    Span::styled(String::from(ch), Style::default().fg(Color::Rgb(0, bright, 60))),
                );
            }
        });

    frame.render_widget(canvas, area);
}

fn render_connection(frame: &mut Frame, app: &App, area: Rect) {
    let status_color = if app.conn_info.connected { Color::Green } else { Color::Red };
    let status_text = if app.conn_info.connected { "CONNECTED" } else { "DISCONNECTED" };
    let uptime = app.uptime_string();

    let ks_color = if app.conn_info.kill_switch.starts_with("ON") {
        Color::Green
    } else if app.conn_info.kill_switch.starts_with("PARTIAL") {
        Color::Yellow
    } else {
        Color::Red
    };

    let text = vec![
        Line::from(vec![
            Span::styled("  Status:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(status_text, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  Server:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.conn_info.server, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  City:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.conn_info.city, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  IP:      ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.conn_info.ip, Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("  Proto:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.conn_info.protocol, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Uptime:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(&uptime, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Kill SW: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.conn_info.kill_switch, Style::default().fg(ks_color)),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Connection ")
        .border_style(Style::default().fg(Color::DarkGray))
        .title_style(Style::default().fg(Color::Cyan));

    frame.render_widget(Paragraph::new(text).block(block), area);
}

fn render_bandwidth(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Bandwidth ")
        .border_style(Style::default().fg(Color::DarkGray))
        .title_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 2 { return; }

    let bw_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    let rx_label = format!(" DL {} ", BandwidthMonitor::format_rate(app.bandwidth.rx_rate));
    let rx_sparkline = Sparkline::default()
        .data(&app.bandwidth.rx_history)
        .style(Style::default().fg(Color::Green))
        .bar_set(ratatui::symbols::bar::NINE_LEVELS);
    let rx_line = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(rx_label.len() as u16)])
        .split(bw_chunks[0]);
    frame.render_widget(rx_sparkline, rx_line[0]);
    frame.render_widget(
        Paragraph::new(rx_label).style(Style::default().fg(Color::Green)),
        rx_line[1],
    );

    if bw_chunks.len() > 1 {
        let tx_label = format!(" UL {} ", BandwidthMonitor::format_rate(app.bandwidth.tx_rate));
        let tx_sparkline = Sparkline::default()
            .data(&app.bandwidth.tx_history)
            .style(Style::default().fg(Color::Cyan))
            .bar_set(ratatui::symbols::bar::NINE_LEVELS);
        let tx_line = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1), Constraint::Length(tx_label.len() as u16)])
            .split(bw_chunks[1]);
        frame.render_widget(tx_sparkline, tx_line[0]);
        frame.render_widget(
            Paragraph::new(tx_label).style(Style::default().fg(Color::Cyan)),
            tx_line[1],
        );
    }
}

fn render_actions(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = MENU_ACTIONS
        .iter()
        .enumerate()
        .map(|(i, (_, label))| {
            let is_selected = Some(i) == app.list_state.selected();
            let style = if is_selected {
                Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let prefix = if is_selected { " > " } else { "   " };
            ListItem::new(format!("{prefix}{label}")).style(style)
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Actions ")
        .border_style(Style::default().fg(Color::DarkGray))
        .title_style(Style::default().fg(Color::Cyan));

    let list = List::new(items).block(block);
    let mut state = app.list_state.clone();
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_network(frame: &mut Frame, app: &App, area: Rect) {
    let dns_color = if app.net_info.dns_safe { Color::Green } else { Color::Red };
    let leak_color = if app.net_info.leak.contains("None") { Color::Green } else { Color::Yellow };

    let text = vec![
        Line::from(vec![
            Span::styled("  SSID:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.net_info.ssid, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Route:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.net_info.route, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  DNS:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.net_info.dns, Style::default().fg(dns_color)),
        ]),
        Line::from(vec![
            Span::styled("  Leak:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.net_info.leak, Style::default().fg(leak_color)),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Network ")
        .border_style(Style::default().fg(Color::DarkGray))
        .title_style(Style::default().fg(Color::Cyan));

    frame.render_widget(Paragraph::new(text).block(block), area);
}

fn render_footer(frame: &mut Frame, area: Rect) {
    let version = env!("CARGO_PKG_VERSION");
    let text = Line::from(vec![
        Span::styled(" j/k ", Style::default().fg(Color::Cyan)),
        Span::styled("navigate  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Enter ", Style::default().fg(Color::Cyan)),
        Span::styled("select  ", Style::default().fg(Color::DarkGray)),
        Span::styled("r ", Style::default().fg(Color::Cyan)),
        Span::styled("refresh  ", Style::default().fg(Color::DarkGray)),
        Span::styled("q ", Style::default().fg(Color::Cyan)),
        Span::styled("quit", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(
                "{:>width$}",
                format!("tuinnel v{version}"),
                width = (area.width as usize).saturating_sub(42)
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    frame.render_widget(Paragraph::new(text), area);
}

// ── Overlay Rendering ───────────────────────────────────────────────────────

fn render_spinner_overlay(
    frame: &mut Frame,
    title: &str,
    spinner_frame: usize,
    started: Instant,
    area: Rect,
) {
    let popup_area = centered_rect(50, 25, area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title))
        .border_style(Style::default().fg(Color::Yellow))
        .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let elapsed = started.elapsed().as_secs();
    let spinner_char = SPINNER[spinner_frame % SPINNER.len()];

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("  {spinner_char} "), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{title}..."), Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(format!("  {elapsed}s elapsed"), Style::default().fg(Color::DarkGray))]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Esc ", Style::default().fg(Color::Cyan)),
            Span::styled("cancel", Style::default().fg(Color::DarkGray)),
        ]),
    ];

    frame.render_widget(Paragraph::new(text), inner);
}

fn render_result_overlay(frame: &mut Frame, overlay: &Overlay, area: Rect) {
    let popup_area = centered_rect(70, 70, area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", overlay.title))
        .border_style(Style::default().fg(Color::Cyan))
        .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let visible_height = inner.height as usize;
    let total_lines = overlay.lines.len();
    let start = overlay.scroll.min(total_lines.saturating_sub(visible_height));
    let content_height = if visible_height > 1 { visible_height - 1 } else { visible_height };
    let content_end = (start + content_height).min(total_lines);

    let visible_lines: Vec<Line> = overlay.lines[start..content_end].to_vec();
    frame.render_widget(Paragraph::new(visible_lines), inner);

    if inner.height > 1 {
        let hint_area = Rect {
            x: inner.x,
            y: inner.y + inner.height - 1,
            width: inner.width,
            height: 1,
        };

        let mut hints = vec![
            Span::styled(" Esc ", Style::default().fg(Color::Cyan)),
            Span::styled("close", Style::default().fg(Color::DarkGray)),
        ];

        if total_lines > content_height {
            hints.push(Span::styled("  j/k ", Style::default().fg(Color::Cyan)));
            hints.push(Span::styled("scroll", Style::default().fg(Color::DarkGray)));
        }

        frame.render_widget(Paragraph::new(Line::from(hints)), hint_area);
    }
}

fn render_picker_overlay(frame: &mut Frame, picker: &PickerState, area: Rect) {
    let popup_area = centered_rect(50, 70, area);
    frame.render_widget(Clear, popup_area);

    let title = match &picker.step {
        PickerStep::Countries => " Select Country ".to_string(),
        PickerStep::Cities { country_name, .. } => format!(" {} — Select City ", country_name),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Cyan))
        .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    if inner.height < 3 { return; }

    let search_area = Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 };
    let search_display = if picker.query.is_empty() {
        Line::from(vec![
            Span::styled(" / ", Style::default().fg(Color::Cyan)),
            Span::styled("type to filter...", Style::default().fg(Color::DarkGray)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" / ", Style::default().fg(Color::Cyan)),
            Span::styled(&picker.query, Style::default().fg(Color::White)),
            Span::styled("_", Style::default().fg(Color::Cyan)),
        ])
    };
    frame.render_widget(Paragraph::new(search_display), search_area);

    let list_height = (inner.height as usize).saturating_sub(2);
    let total = picker.filtered.len();

    let scroll = if picker.selected < picker.scroll_offset {
        picker.selected
    } else if picker.selected >= picker.scroll_offset + list_height {
        picker.selected - list_height + 1
    } else {
        picker.scroll_offset
    };

    let end = (scroll + list_height).min(total);

    let items: Vec<Line> = if total == 0 {
        vec![Line::from(Span::styled("   no matches", Style::default().fg(Color::DarkGray)))]
    } else {
        picker.filtered[scroll..end]
            .iter()
            .enumerate()
            .map(|(i, &idx)| {
                let entry = &picker.all_entries[idx];
                let abs = scroll + i;
                if abs == picker.selected {
                    Line::from(Span::styled(
                        format!(" > {} ", entry.display),
                        Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(Span::styled(
                        format!("   {} ", entry.display),
                        Style::default().fg(Color::White),
                    ))
                }
            })
            .collect()
    };

    let list_area = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: inner.height.saturating_sub(2),
    };
    frame.render_widget(Paragraph::new(items), list_area);

    let hint_area = Rect {
        x: inner.x,
        y: inner.y + inner.height - 1,
        width: inner.width,
        height: 1,
    };
    let back_hint = match &picker.step {
        PickerStep::Countries => "close",
        PickerStep::Cities { .. } => "back",
    };
    let count_hint = format!(" {}/{} ", total, picker.all_entries.len());
    let hints = Line::from(vec![
        Span::styled(" Esc ", Style::default().fg(Color::Cyan)),
        Span::styled(back_hint, Style::default().fg(Color::DarkGray)),
        Span::styled("  ↑↓ ", Style::default().fg(Color::Cyan)),
        Span::styled("navigate", Style::default().fg(Color::DarkGray)),
        Span::styled("  Enter ", Style::default().fg(Color::Cyan)),
        Span::styled("select", Style::default().fg(Color::DarkGray)),
        Span::styled(&count_hint, Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(hints), hint_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

// ── Action Handling ─────────────────────────────────────────────────────────

fn handle_selection(app: &mut App) {
    if matches!(app.overlay, OverlayState::Running { .. }) {
        return;
    }

    let action = app.selected_action();

    match action {
        MenuAction::Quit => {
            app.should_quit = true;
            return;
        }
        MenuAction::Refresh => {
            app.spawn_stats_refresh();
            return;
        }
        _ => {}
    }

    let title = match action {
        MenuAction::Reconnect => "Reconnect",
        MenuAction::ChangeServer => "Change Server",
        MenuAction::Disconnect => "Disconnect",
        MenuAction::KillSwitch => "Kill Switch",
        MenuAction::SecurityAudit => "Security Audit",
        MenuAction::Doctor => "Doctor",
        _ => return,
    };

    match action {
        MenuAction::ChangeServer => {
            // Instant — built from local server list, no background thread
            let entries: Vec<PickerEntry> = servers::countries(&app.servers)
                .into_iter()
                .map(|(code, _)| {
                    let count = servers::find_by_country(&app.servers, &code).len();
                    PickerEntry {
                        display: format!("{code}  ({count} server(s))"),
                        value: code,
                    }
                })
                .collect();

            if entries.is_empty() {
                let mut buf = OutputBuffer::new();
                buf.warn("No servers found. Drop .conf files into ~/.config/tuinnel/servers/");
                app.overlay = OverlayState::Done(Overlay {
                    title: title.into(),
                    lines: buf.to_ratatui_lines(),
                    scroll: 0,
                });
            } else {
                app.overlay = OverlayState::Picker(PickerState::new(
                    PickerStep::Countries, entries,
                ));
            }
            return;
        }
        _ => {}
    }

    // Show spinner for background actions
    app.overlay = OverlayState::Running {
        title: title.to_string(),
        spinner_frame: 0,
        started: Instant::now(),
    };

    let backend = Arc::clone(&app.backend);
    let config = Arc::clone(&app.config);
    let tx = app.msg_tx.clone();
    let session = app.session.clone();
    let server_list = app.servers.clone();

    match action {
        MenuAction::Reconnect => {
            thread::spawn(move || {
                let target = match config.general.default_connect.as_str() {
                    "random" => ConnectTarget::Random,
                    "preferred" => ConnectTarget::Preferred,
                    _ => ConnectTarget::First,
                };
                // Resolve target to a server
                let manager_tmp = VpnManager::new(backend.clone(), server_list);
                let server = manager_tmp.resolve_target(&target, &config);
                let result = match server {
                    Some(s) => backend.connect(&s),
                    None => Err("No server found for reconnect".into()),
                };
                let _ = tx.send(AppMessage::ConnectResult { result });
            });
        }
        MenuAction::Disconnect => {
            thread::spawn(move || {
                let result = match session {
                    Some(ref s) => backend.disconnect(s),
                    None => Err("Not connected".into()),
                };
                let _ = tx.send(AppMessage::DisconnectResult { result });
            });
        }
        MenuAction::KillSwitch => {
            thread::spawn(move || {
                let mut buf = OutputBuffer::new();
                buf.header("Kill Switch");
                buf.warn("Native kill switch not yet implemented (coming soon)");
                let _ = tx.send(AppMessage::ActionDone {
                    title: "Kill Switch".into(),
                    buf,
                });
            });
        }
        MenuAction::SecurityAudit => {
            thread::spawn(move || {
                let buf = security::audit_report(session.as_ref());
                let _ = tx.send(AppMessage::ActionDone {
                    title: "Security Audit".into(),
                    buf,
                });
            });
        }
        MenuAction::Doctor => {
            thread::spawn(move || {
                let manager_tmp = VpnManager::new(backend, server_list);
                let buf = crate::doctor::run(&manager_tmp, &config, session.as_ref());
                let _ = tx.send(AppMessage::ActionDone {
                    title: "Doctor".into(),
                    buf,
                });
            });
        }
        _ => {
            app.overlay = OverlayState::None;
        }
    }
}

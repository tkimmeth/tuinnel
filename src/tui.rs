// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// tui.rs — Full-screen dashboard TUI with spinning globe + overlay system
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Layout:
//   ┌─ pvpn ────────────────────────────────────────────────────────┐
//   │  [Globe]          │  [Connection Info]                        │
//   │  Spinning braille │  Server, City, IP, Protocol, Uptime, KS  │
//   │  globe with       ├──────────────────────────────────────────│
//   │  server marker    │  [Bandwidth]                              │
//   │                   │  RX/TX sparklines                         │
//   ├───────────────────┼──────────────────────────────────────────│
//   │  [Actions]        │  [Network]                                │
//   │  > Reconnect      │  SSID, Route, DNS, Leak                  │
//   │    Change Server  │                                           │
//   │    Disconnect     │                                           │
//   └─ j/k  Enter  r  q ─────────────────────── pvpn v0.1.0 ─────┘
//
// All blocking operations (connect, disconnect, security audit, etc.) run
// in background threads. The UI stays responsive with a spinner overlay
// while work is in progress. Results arrive via mpsc channels.

use crate::bandwidth::BandwidthMonitor;
use crate::commands;
use crate::config::Config;
use crate::geo;
use crate::globe::{self, WorldMap};
use crate::net;
use crate::output::OutputBuffer;
use crate::security;
use crate::vpn::{ConnectMode, ProtonVPN};

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
    StatsRefreshed {
        conn_info: ConnectionInfo,
        net_info: NetworkInfo,
    },
    CountriesFetched {
        entries: Vec<PickerEntry>,
    },
    CitiesFetched {
        country_code: String,
        country_name: String,
        entries: Vec<PickerEntry>,
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
    filtered: Vec<usize>, // indices into all_entries
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

/// Simple fuzzy match: all chars of needle appear in haystack in order.
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
    dns_is_proton: bool,
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

    // Shared state for background threads
    vpn: Arc<ProtonVPN>,
    config: Arc<Config>,
    msg_tx: mpsc::Sender<AppMessage>,
    msg_rx: mpsc::Receiver<AppMessage>,
    stats_refresh_pending: bool,

    // Server picker cache
    cached_countries: Option<Vec<PickerEntry>>,
}

impl App {
    fn new(vpn: Arc<ProtonVPN>, config: Arc<Config>) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        let world_map = WorldMap::generate();
        let conn_info = Self::gather_connection_info(&vpn);
        let net_info = Self::gather_network_info();
        let bandwidth = BandwidthMonitor::new();

        let server_coords = conn_info
            .server
            .as_str()
            .ne("--")
            .then(|| geo::lookup_server(&conn_info.server))
            .flatten();

        let connect_time = if conn_info.connected {
            Some(Instant::now())
        } else {
            None
        };

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
            vpn,
            config,
            msg_tx,
            msg_rx,
            stats_refresh_pending: false,
            cached_countries: None,
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

    fn gather_connection_info(vpn: &ProtonVPN) -> ConnectionInfo {
        let status_text = vpn.status().unwrap_or_default();
        let lower = status_text.to_lowercase();
        let connected = lower.contains("connected") && !lower.contains("disconnected");

        let mut server = "--".to_string();
        let mut city = "--".to_string();
        let protocol;

        if connected {
            for line in status_text.lines() {
                let trimmed = line.trim();
                if trimmed.contains("Name:") {
                    if let Some(name) = trimmed.split("Name:").nth(1) {
                        // Clean up: remove "Device: ..." suffix from nmcli output
                        let name = name.split("Device:").next().unwrap_or(name).trim();
                        if name.to_lowercase().contains("proton")
                            || name.contains('-')
                            || name.contains('#')
                        {
                            // Extract server ID (e.g. "US-CA#295" from "ProtonVPN US-CA#295")
                            let server_id = name
                                .split_whitespace()
                                .find(|w| w.contains('#'))
                                .unwrap_or(name);
                            server = server_id.to_string();
                            if let Some(city_name) = geo::server_city_name(server_id) {
                                city = city_name.to_string();
                            }
                        }
                    }
                }
            }

            let (tunneled, route) = net::is_tunneled();
            if route.contains("wg") || route.contains("proton0") {
                protocol = "WireGuard".to_string();
            } else if route.contains("tun") {
                protocol = "OpenVPN".to_string();
            } else if tunneled {
                protocol = "VPN tunnel".to_string();
            } else {
                protocol = "--".to_string();
            }
        } else {
            protocol = "--".to_string();
        }

        let ip = if connected {
            net::public_ip()
        } else {
            "--".to_string()
        };

        // Kill switch — enhanced with firewall verification
        let kill_switch = if vpn.caps.kill_switch {
            match vpn.kill_switch("status") {
                Ok(s) => {
                    let is_on =
                        s.to_lowercase().contains("on") || s.to_lowercase().contains("active");
                    if is_on {
                        let fw_check = security::check_kill_switch_firewall();
                        if fw_check.passed {
                            format!("ON ({})", fw_check.detail)
                        } else {
                            "PARTIAL - no FW rules".to_string()
                        }
                    } else {
                        "OFF".to_string()
                    }
                }
                Err(_) => "unknown".to_string(),
            }
        } else {
            "N/A".to_string()
        };

        ConnectionInfo {
            connected,
            server,
            city,
            ip,
            protocol,
            kill_switch,
        }
    }

    fn gather_network_info() -> NetworkInfo {
        let ssid = net::wifi_ssid();

        let route_raw = net::default_route();
        let route = route_raw
            .lines()
            .next()
            .unwrap_or("--")
            .to_string();

        // DNS with ProtonDNS detection
        let dns_raw = net::dns_servers();
        let dns_line = dns_raw
            .lines()
            .filter(|l| l.contains("nameserver") || l.contains("10.") || l.contains("DNS"))
            .map(|l| l.trim())
            .next()
            .unwrap_or_else(|| dns_raw.lines().next().unwrap_or("--"))
            .to_string();

        let dns_check = security::check_dns_leak();
        let dns_is_proton = dns_check.passed;

        let dns = if dns_is_proton {
            format!("{dns_line} (ProtonDNS)")
        } else {
            dns_line
        };

        // Comprehensive leak check
        let (tunneled, _) = net::is_tunneled();
        let ipv6_check = security::check_ipv6_leak();

        let leak = if tunneled && dns_is_proton && ipv6_check.passed {
            "None detected".to_string()
        } else {
            let mut issues = Vec::new();
            if !tunneled {
                issues.push("traffic not tunneled");
            }
            if !dns_is_proton {
                issues.push("DNS leak");
            }
            if !ipv6_check.passed {
                issues.push("IPv6 leak");
            }
            if issues.is_empty() {
                "None detected".to_string()
            } else {
                format!("WARNING: {}", issues.join(", "))
            }
        };

        NetworkInfo {
            ssid,
            route,
            dns,
            dns_is_proton,
            leak,
        }
    }

    /// Spawn a background thread to refresh VPN/network stats.
    fn spawn_stats_refresh(&mut self) {
        if self.stats_refresh_pending {
            return;
        }
        self.stats_refresh_pending = true;
        let vpn = Arc::clone(&self.vpn);
        let tx = self.msg_tx.clone();
        thread::spawn(move || {
            let conn_info = App::gather_connection_info(&vpn);
            let net_info = App::gather_network_info();
            let _ = tx.send(AppMessage::StatsRefreshed { conn_info, net_info });
        });
    }

    /// Apply stats received from a background refresh.
    fn apply_stats_refresh(&mut self, conn_info: ConnectionInfo, net_info: NetworkInfo) {
        self.conn_info = conn_info;
        self.net_info = net_info;
        self.bandwidth.refresh_interface();

        self.server_coords = if self.conn_info.server != "--" {
            geo::lookup_server(&self.conn_info.server)
        } else {
            None
        };

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

pub fn run(config: Arc<Config>, vpn: Arc<ProtonVPN>) -> anyhow::Result<()> {
    if !io::stdout().is_terminal() {
        anyhow::bail!("TUI requires a terminal (TTY)");
    }

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(vpn, config);

    let result = main_loop(&mut terminal, &mut app);

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
                    // Refresh stats after any action completes
                    app.spawn_stats_refresh();
                }
                AppMessage::StatsRefreshed { conn_info, net_info } => {
                    app.apply_stats_refresh(conn_info, net_info);
                }
                AppMessage::CountriesFetched { entries } => {
                    if entries.is_empty() {
                        app.overlay = OverlayState::Done(Overlay {
                            title: "Change Server".into(),
                            lines: vec![Line::from(Span::styled(
                                "  No countries available",
                                Style::default().fg(Color::Red),
                            ))],
                            scroll: 0,
                        });
                    } else {
                        app.cached_countries = Some(entries.clone());
                        app.overlay = OverlayState::Picker(PickerState::new(
                            PickerStep::Countries, entries,
                        ));
                    }
                }
                AppMessage::CitiesFetched { country_code, country_name, entries } => {
                    if entries.is_empty() {
                        // No cities — connect directly to country
                        app.overlay = OverlayState::Running {
                            title: format!("Connecting to {country_name}"),
                            spinner_frame: 0,
                            started: Instant::now(),
                        };
                        let vpn = Arc::clone(&app.vpn);
                        let config = Arc::clone(&app.config);
                        let tx = app.msg_tx.clone();
                        thread::spawn(move || {
                            let buf = commands::do_connect(&vpn, &config, ConnectMode::Country(country_code));
                            let _ = tx.send(AppMessage::ActionDone {
                                title: "Change Server".into(),
                                buf,
                            });
                        });
                    } else {
                        app.overlay = OverlayState::Picker(PickerState::new(
                            PickerStep::Cities { country_code, country_name }, entries,
                        ));
                    }
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
    // Handle picker separately to avoid borrow issues
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
                    app.overlay = OverlayState::Running {
                        title: format!("Loading {name}"),
                        spinner_frame: 0,
                        started: Instant::now(),
                    };
                    let vpn = Arc::clone(&app.vpn);
                    let tx = app.msg_tx.clone();
                    thread::spawn(move || {
                        match vpn.list_cities(&cc) {
                            Ok(cities) => {
                                let entries = cities.into_iter()
                                    .map(|c| PickerEntry { display: c.clone(), value: c })
                                    .collect();
                                let _ = tx.send(AppMessage::CitiesFetched {
                                    country_code: cc,
                                    country_name: name,
                                    entries,
                                });
                            }
                            Err(e) => {
                                let mut buf = OutputBuffer::new();
                                buf.err(&format!("Failed to load cities: {e}"));
                                let _ = tx.send(AppMessage::ActionDone {
                                    title: "Change Server".into(),
                                    buf,
                                });
                            }
                        }
                    });
                }
                PickerStep::Cities { .. } => {
                    let city = entry.value.clone();
                    app.overlay = OverlayState::Running {
                        title: format!("Connecting to {city}"),
                        spinner_frame: 0,
                        started: Instant::now(),
                    };
                    let vpn = Arc::clone(&app.vpn);
                    let config = Arc::clone(&app.config);
                    let tx = app.msg_tx.clone();
                    thread::spawn(move || {
                        let buf = commands::do_connect(&vpn, &config, ConnectMode::City(city));
                        let _ = tx.send(AppMessage::ActionDone {
                            title: "Change Server".into(),
                            buf,
                        });
                    });
                }
            }
        }
        KeyCode::Esc => {
            let go_back_to_countries = matches!(picker.step, PickerStep::Cities { .. });
            if go_back_to_countries {
                if let Some(cached) = app.cached_countries.clone() {
                    app.overlay = OverlayState::Picker(PickerState::new(
                        PickerStep::Countries, cached,
                    ));
                } else {
                    app.overlay = OverlayState::None;
                }
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

    // Outer border
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(" pvpn ")
        .title_alignment(Alignment::Left)
        .border_style(Style::default().fg(Color::Cyan))
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // Main vertical split: top panels | bottom panels | footer
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(55),
            Constraint::Percentage(40),
            Constraint::Length(1),
        ])
        .split(inner);

    // Top: Globe (left) | Connection + Bandwidth (right)
    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(main_chunks[0]);

    // Right panels: Connection (top) | Bandwidth (bottom)
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(top_chunks[1]);

    // Bottom: Actions (left) | Network (right)
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

    // Draw overlay on top of everything if active
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

    // User location from config (None if not configured — no marker shown)
    let user_coords = app.config.general.user_lat
        .zip(app.config.general.user_lon);

    let user_point = user_coords.and_then(|(lat, lon)| {
        globe::project_point(lat, lon, app.globe_rotation)
    });

    let server_point = app.server_coords.and_then(|(lat, lon)| {
        globe::project_point(lat, lon, app.globe_rotation)
    });

    // Arc from user (or US center fallback) to server
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

            // Arc — solid braille line + animated flow dots
            if let Some(ref arc) = arc_points {
                if arc.len() >= 2 {
                    // Solid line via braille
                    ctx.draw(&Points {
                        coords: arc,
                        color: Color::Rgb(0, 130, 60),
                    });
                    // Animated bright dots flowing along it
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

            // User location
            if let Some((ux, uy)) = user_point {
                let ch = if tick / 6 % 2 == 0 { '◆' } else { '◇' };
                ctx.print(ux, uy,
                    Span::styled(String::from(ch), Style::default().fg(Color::Rgb(0, 255, 100))),
                );
            }

            // Server location — pulsing
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
    let status_color = if app.conn_info.connected {
        Color::Green
    } else {
        Color::Red
    };
    let status_text = if app.conn_info.connected {
        "CONNECTED"
    } else {
        "DISCONNECTED"
    };

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
            Span::styled(
                status_text,
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
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

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
}

fn render_bandwidth(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Bandwidth ")
        .border_style(Style::default().fg(Color::DarkGray))
        .title_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 2 {
        return;
    }

    let bw_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    // RX sparkline
    let rx_label = format!(" DL {} ", BandwidthMonitor::format_rate(app.bandwidth.rx_rate));
    let rx_sparkline = Sparkline::default()
        .data(&app.bandwidth.rx_history)
        .style(Style::default().fg(Color::Green))
        .bar_set(ratatui::symbols::bar::NINE_LEVELS);
    let rx_line = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(rx_label.len() as u16),
        ])
        .split(bw_chunks[0]);
    frame.render_widget(rx_sparkline, rx_line[0]);
    frame.render_widget(
        Paragraph::new(rx_label).style(Style::default().fg(Color::Green)),
        rx_line[1],
    );

    // TX sparkline
    if bw_chunks.len() > 1 {
        let tx_label = format!(" UL {} ", BandwidthMonitor::format_rate(app.bandwidth.tx_rate));
        let tx_sparkline = Sparkline::default()
            .data(&app.bandwidth.tx_history)
            .style(Style::default().fg(Color::Cyan))
            .bar_set(ratatui::symbols::bar::NINE_LEVELS);
        let tx_line = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(tx_label.len() as u16),
            ])
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
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
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
    let dns_color = if app.net_info.dns_is_proton {
        Color::Green
    } else {
        Color::Red
    };

    let leak_color = if app.net_info.leak.contains("None") {
        Color::Green
    } else {
        Color::Yellow
    };

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

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
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
                format!("pvpn v{version}"),
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
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let elapsed = started.elapsed().as_secs();
    let spinner_char = SPINNER[spinner_frame % SPINNER.len()];

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                format!("  {spinner_char} "),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{title}..."),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            format!("  {elapsed}s elapsed"),
            Style::default().fg(Color::DarkGray),
        )]),
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
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Content with scroll support
    let visible_height = inner.height as usize;
    let total_lines = overlay.lines.len();
    let start = overlay.scroll.min(total_lines.saturating_sub(visible_height));

    // Reserve last line for hint
    let content_height = if visible_height > 1 {
        visible_height - 1
    } else {
        visible_height
    };
    let content_end = (start + content_height).min(total_lines);

    let visible_lines: Vec<Line> = overlay.lines[start..content_end].to_vec();
    let paragraph = Paragraph::new(visible_lines);
    frame.render_widget(paragraph, inner);

    // Footer hint at bottom of popup
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

    // Search bar at top
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

    // List area (between search bar and footer)
    let list_height = (inner.height as usize).saturating_sub(2); // search + footer
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
        vec![Line::from(Span::styled(
            "   no matches",
            Style::default().fg(Color::DarkGray),
        ))]
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

    // Footer hints
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

/// Compute a centered rectangle within `r` at the given percentage size.
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
    // Block new actions while one is already running
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

    // Show spinner immediately
    let title = match action {
        MenuAction::Reconnect => "Reconnect",
        MenuAction::ChangeServer => "Change Server",
        MenuAction::Disconnect => "Disconnect",
        MenuAction::KillSwitch => "Kill Switch",
        MenuAction::SecurityAudit => "Security Audit",
        MenuAction::Doctor => "Doctor",
        _ => return,
    };

    app.overlay = OverlayState::Running {
        title: title.to_string(),
        spinner_frame: 0,
        started: Instant::now(),
    };

    // Spawn blocking work in a background thread
    let vpn = Arc::clone(&app.vpn);
    let config = Arc::clone(&app.config);
    let tx = app.msg_tx.clone();

    match action {
        MenuAction::Reconnect => {
            thread::spawn(move || {
                let mode = match config.general.default_connect.as_str() {
                    "random" => ConnectMode::Random,
                    "preferred" => ConnectMode::Preferred,
                    _ => ConnectMode::Fastest,
                };
                let buf = commands::do_connect(&vpn, &config, mode);
                let _ = tx.send(AppMessage::ActionDone {
                    title: "Reconnect".into(),
                    buf,
                });
            });
        }
        MenuAction::ChangeServer => {
            thread::spawn(move || {
                match vpn.list_countries() {
                    Ok(countries) => {
                        let entries = countries.into_iter()
                            .map(|(name, code)| PickerEntry { display: format!("{name}  ({code})"), value: code })
                            .collect();
                        let _ = tx.send(AppMessage::CountriesFetched { entries });
                    }
                    Err(e) => {
                        let mut buf = OutputBuffer::new();
                        buf.err(&format!("Failed to load countries: {e}"));
                        let _ = tx.send(AppMessage::ActionDone {
                            title: "Change Server".into(),
                            buf,
                        });
                    }
                }
            });
        }
        MenuAction::Disconnect => {
            thread::spawn(move || {
                let buf = commands::do_disconnect(&vpn);
                let _ = tx.send(AppMessage::ActionDone {
                    title: "Disconnect".into(),
                    buf,
                });
            });
        }
        MenuAction::KillSwitch => {
            thread::spawn(move || {
                let current = vpn.kill_switch("status");
                let action_str = match current {
                    Ok(ref s) if s.to_lowercase().contains("on") => "off",
                    _ => "on",
                };
                let title = format!("Kill Switch ({action_str})");
                let buf = commands::do_ks(&vpn, action_str);
                let _ = tx.send(AppMessage::ActionDone { title, buf });
            });
        }
        MenuAction::SecurityAudit => {
            thread::spawn(move || {
                let buf = security::audit_report();
                let _ = tx.send(AppMessage::ActionDone {
                    title: "Security Audit".into(),
                    buf,
                });
            });
        }
        MenuAction::Doctor => {
            thread::spawn(move || {
                let buf = crate::doctor::run(&vpn, &config);
                let _ = tx.send(AppMessage::ActionDone {
                    title: "Doctor".into(),
                    buf,
                });
            });
        }
        _ => {
            // Refresh and Quit handled above
            app.overlay = OverlayState::None;
        }
    }
}

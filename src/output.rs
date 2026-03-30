// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// output.rs — Colored terminal output helpers + structured output buffer
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Two output paths:
//
//   1. Free functions (header, ok, warn, etc.) — print directly to stdout.
//      Used by the CLI code path.
//
//   2. OutputBuffer — captures styled lines for later rendering.
//      Used by the TUI overlay so command output stays inside the dashboard.

use ratatui::prelude::*;
use ratatui::text::{Line, Span};

// ── ANSI Constants (for CLI direct output) ──────────────────────────────────

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const RED: &str = "\x1b[31m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const CYAN: &str = "\x1b[36m";

// ── CLI Free Functions ──────────────────────────────────────────────────────
//
// These are kept for direct CLI usage outside the OutputBuffer system
// (e.g., quick one-off messages that don't need to be captured).

#[allow(dead_code)]
pub fn header(title: &str) {
    println!("\n{BOLD}{CYAN}── {title} ──{RESET}");
}

#[allow(dead_code)]
pub fn ok(msg: &str) {
    println!("  {GREEN}✓{RESET} {msg}");
}

#[allow(dead_code)]
pub fn warn(msg: &str) {
    println!("  {YELLOW}⚠{RESET} {msg}");
}

#[allow(dead_code)]
pub fn err(msg: &str) {
    println!("  {RED}✗{RESET} {msg}");
}

#[allow(dead_code)]
pub fn kv(key: &str, val: &str) {
    println!("  {BOLD}{key}:{RESET} {val}");
}

#[allow(dead_code)]
pub fn indent(msg: &str) {
    println!("    {msg}");
}

#[allow(dead_code)]
pub fn dim(msg: &str) {
    println!("  {DIM}{msg}{RESET}");
}

// ── Structured Output Buffer ────────────────────────────────────────────────

/// Semantic kind of an output line — determines how it's colored.
#[derive(Clone, Debug)]
pub enum LineKind {
    Header(String),
    Ok(String),
    Warn(String),
    Err(String),
    Kv(String, String), // (key, value)
    Indent(String),
    Dim(String),
    Plain(String),
    Blank,
}

/// Collects styled output lines for later rendering in CLI or TUI.
#[derive(Clone, Debug, Default)]
pub struct OutputBuffer {
    pub lines: Vec<LineKind>,
}

impl OutputBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn header(&mut self, title: &str) {
        self.lines.push(LineKind::Blank);
        self.lines.push(LineKind::Header(title.to_string()));
    }

    pub fn ok(&mut self, msg: &str) {
        self.lines.push(LineKind::Ok(msg.to_string()));
    }

    pub fn warn(&mut self, msg: &str) {
        self.lines.push(LineKind::Warn(msg.to_string()));
    }

    pub fn err(&mut self, msg: &str) {
        self.lines.push(LineKind::Err(msg.to_string()));
    }

    pub fn kv(&mut self, key: &str, val: &str) {
        self.lines
            .push(LineKind::Kv(key.to_string(), val.to_string()));
    }

    pub fn indent(&mut self, msg: &str) {
        self.lines.push(LineKind::Indent(msg.to_string()));
    }

    pub fn dim(&mut self, msg: &str) {
        self.lines.push(LineKind::Dim(msg.to_string()));
    }

    pub fn plain(&mut self, msg: &str) {
        self.lines.push(LineKind::Plain(msg.to_string()));
    }

    pub fn blank(&mut self) {
        self.lines.push(LineKind::Blank);
    }

    // ── Rendering ───────────────────────────────────────────────────────

    /// Print all lines to stdout using ANSI escape codes (CLI path).
    pub fn print_all(&self) {
        for kind in &self.lines {
            match kind {
                LineKind::Header(t) => println!("\n{BOLD}{CYAN}── {t} ──{RESET}"),
                LineKind::Ok(m) => println!("  {GREEN}✓{RESET} {m}"),
                LineKind::Warn(m) => println!("  {YELLOW}⚠{RESET} {m}"),
                LineKind::Err(m) => println!("  {RED}✗{RESET} {m}"),
                LineKind::Kv(k, v) => println!("  {BOLD}{k}:{RESET} {v}"),
                LineKind::Indent(m) => println!("    {m}"),
                LineKind::Dim(m) => println!("  {DIM}{m}{RESET}"),
                LineKind::Plain(m) => println!("  {m}"),
                LineKind::Blank => println!(),
            }
        }
    }

    /// Convert to ratatui Lines for rendering inside the TUI overlay.
    pub fn to_ratatui_lines(&self) -> Vec<Line<'static>> {
        self.lines.iter().map(|kind| kind.to_ratatui_line()).collect()
    }
}

impl LineKind {
    /// Convert a single styled line to a ratatui Line.
    pub fn to_ratatui_line(&self) -> Line<'static> {
        match self {
            LineKind::Header(t) => Line::from(vec![
                Span::styled(
                    format!(" ── {t} ──"),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            LineKind::Ok(m) => Line::from(vec![
                Span::styled("  ✓ ", Style::default().fg(Color::Green)),
                Span::styled(m.clone(), Style::default().fg(Color::White)),
            ]),
            LineKind::Warn(m) => Line::from(vec![
                Span::styled("  ⚠ ", Style::default().fg(Color::Yellow)),
                Span::styled(m.clone(), Style::default().fg(Color::Yellow)),
            ]),
            LineKind::Err(m) => Line::from(vec![
                Span::styled("  ✗ ", Style::default().fg(Color::Red)),
                Span::styled(m.clone(), Style::default().fg(Color::Red)),
            ]),
            LineKind::Kv(k, v) => Line::from(vec![
                Span::styled(
                    format!("  {k}: "),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(v.clone(), Style::default().fg(Color::White)),
            ]),
            LineKind::Indent(m) => Line::from(vec![Span::styled(
                format!("    {m}"),
                Style::default().fg(Color::White),
            )]),
            LineKind::Dim(m) => Line::from(vec![Span::styled(
                format!("  {m}"),
                Style::default().fg(Color::DarkGray),
            )]),
            LineKind::Plain(m) => Line::from(vec![Span::styled(
                format!("  {m}"),
                Style::default().fg(Color::White),
            )]),
            LineKind::Blank => Line::from(""),
        }
    }
}

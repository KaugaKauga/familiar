//! Interactive TUI dashboard for Guild.
//!
//! Displays a live view of all active pipelines with progress bars,
//! stage indicators, and a fun ASCII art banner on startup.

use std::io::{self, stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Wrap};
use ratatui::Terminal;

use crate::pipeline::{Pipeline, Stage};

// ---------------------------------------------------------------------------
// Dashboard state (shared between daemon task and TUI)
// ---------------------------------------------------------------------------

/// A snapshot view of a single pipeline for the TUI to render.
#[derive(Debug, Clone)]
pub struct PipelineView {
    pub issue_number: u64,
    pub title: String,
    pub stage: Stage,
    pub is_failed: bool,
}

impl From<&Pipeline> for PipelineView {
    fn from(p: &Pipeline) -> Self {
        Self {
            issue_number: p.issue_number,
            title: if p.issue_title.is_empty() {
                format!("Issue #{}", p.issue_number)
            } else {
                p.issue_title.clone()
            },
            stage: p.stage.clone(),
            is_failed: p.is_failed(),
        }
    }
}

/// Shared state between the daemon poll loop and the TUI renderer.
#[derive(Debug)]
pub struct DashboardState {
    pub repo: String,
    pub label: String,
    pub poll_interval: u64,
    pub start_time: Instant,
    pub pipelines: Vec<PipelineView>,
    pub last_poll: Option<Instant>,
    pub error_message: Option<String>,
}

impl DashboardState {
    pub fn new(repo: String, label: String, poll_interval: u64) -> Self {
        Self {
            repo,
            label,
            poll_interval,
            start_time: Instant::now(),
            pipelines: Vec::new(),
            last_poll: None,
            error_message: None,
        }
    }

    pub fn uptime_string(&self) -> String {
        let elapsed = self.start_time.elapsed();
        let secs = elapsed.as_secs();
        let hrs = secs / 3600;
        let mins = (secs % 3600) / 60;
        let s = secs % 60;
        if hrs > 0 {
            format!("{}h {:02}m {:02}s", hrs, mins, s)
        } else {
            format!("{}m {:02}s", mins, s)
        }
    }
}

// ---------------------------------------------------------------------------
// ASCII Art Banner
// ---------------------------------------------------------------------------

pub fn display_banner() {
    let mut out = stdout();

    let banner = r#"

                          ░▒▓█ GUILD █▓▒░

          ⚒                                              ⚒
               ╔══════════════════════════════════╗
          ╔════╣      THE  AUTONOMOUS  SOFTWARE   ╠════╗
          ║    ╚══════╦═══════════════════╦══════╝    ║
          ║           ║     F A C T O R Y ║           ║
          ║     ╔═════╩═══════════════════╩═════╗     ║
          ║     ║                               ║     ║
          ║     ║    ┌───┐  ┌───┐  ┌───┐       ║     ║
          ║     ║    │ ⚙ │──│ ⚙ │──│ ⚙ │       ║     ║
          ║     ║    └─┬─┘  └─┬─┘  └─┬─┘       ║     ║
          ║     ║      │      │      │          ║     ║
          ║     ║    ╔═╧══════╧══════╧═╗        ║     ║
          ║     ║    ║  ░░ FORGE ░░░░  ║        ║     ║
          ║     ║    ║  ▓▓▓█████▓▓▓▓▓  ║        ║     ║
          ║     ║    ╚═════════════════╝        ║     ║
          ║     ║         /  |  \               ║     ║
          ║     ║        🔥 🔥 🔥              ║     ║
          ║     ╚═══════════════════════════════╝     ║
          ║                                           ║
          ╚═══════════════════════════════════════════╝

         ┌─────────────────────────────────────────────┐
         │  "We build while you sleep"  — The Guildsmen │
         └─────────────────────────────────────────────┘
"#;

    // Print with colors
    let _ = execute!(out, SetForegroundColor(Color::DarkYellow));
    let _ = execute!(out, Print("\n"));

    for line in banner.lines() {
        if line.contains("GUILD") && line.contains("░▒▓█") {
            let _ = execute!(out, SetForegroundColor(Color::Yellow));
            let _ = execute!(out, Print(format!("{}\n", line)));
            let _ = execute!(out, SetForegroundColor(Color::DarkYellow));
        } else if line.contains("FORGE") || line.contains("▓▓▓") {
            let _ = execute!(out, SetForegroundColor(Color::Red));
            let _ = execute!(out, Print(format!("{}\n", line)));
            let _ = execute!(out, SetForegroundColor(Color::DarkYellow));
        } else if line.contains("🔥") {
            let _ = execute!(out, SetForegroundColor(Color::DarkRed));
            let _ = execute!(out, Print(format!("{}\n", line)));
            let _ = execute!(out, SetForegroundColor(Color::DarkYellow));
        } else if line.contains("⚙") {
            let _ = execute!(out, SetForegroundColor(Color::Cyan));
            let _ = execute!(out, Print(format!("{}\n", line)));
            let _ = execute!(out, SetForegroundColor(Color::DarkYellow));
        } else if line.contains("We build while you sleep") {
            let _ = execute!(out, SetForegroundColor(Color::Green));
            let _ = execute!(out, Print(format!("{}\n", line)));
            let _ = execute!(out, SetForegroundColor(Color::DarkYellow));
        } else if line.contains("AUTONOMOUS") || line.contains("F A C T O R Y") {
            let _ = execute!(out, SetForegroundColor(Color::White));
            let _ = execute!(out, Print(format!("{}\n", line)));
            let _ = execute!(out, SetForegroundColor(Color::DarkYellow));
        } else {
            let _ = execute!(out, Print(format!("{}\n", line)));
        }
    }

    let _ = execute!(out, ResetColor);
    let _ = out.flush();
}

// ---------------------------------------------------------------------------
// TUI Dashboard
// ---------------------------------------------------------------------------

pub fn run_tui(state: Arc<Mutex<DashboardState>>, shutdown: Arc<AtomicBool>) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let tick_rate = Duration::from_millis(500);

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        terminal.draw(|f| {
            let state = state.lock().unwrap();
            render_dashboard(f, &state);
        })?;

        // Poll for events with timeout
        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => {
                            shutdown.store(true, Ordering::SeqCst);
                            break;
                        }
                        KeyCode::Esc => {
                            shutdown.store(true, Ordering::SeqCst);
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

fn render_dashboard(f: &mut ratatui::Frame, state: &DashboardState) {
    let size = f.area();

    // Layout: Header (3), Pipelines (dynamic), Footer (3)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // header
            Constraint::Min(10),   // pipelines
            Constraint::Length(3), // footer
        ])
        .split(size);

    render_header(f, chunks[0], state);
    render_pipelines(f, chunks[1], state);
    render_footer(f, chunks[2], state);
}

fn render_header(f: &mut ratatui::Frame, area: Rect, state: &DashboardState) {
    let uptime = state.uptime_string();
    let last_poll = state
        .last_poll
        .map(|t| format!("{}s ago", t.elapsed().as_secs()))
        .unwrap_or_else(|| "never".to_string());

    let header_text = vec![
        Line::from(vec![
            Span::styled(
                " ⚒  GUILD ",
                Style::default()
                    .fg(ratatui::style::Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" — "),
            Span::styled(
                &state.repo,
                Style::default()
                    .fg(ratatui::style::Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw(" Label: "),
            Span::styled(
                &state.label,
                Style::default().fg(ratatui::style::Color::Magenta),
            ),
            Span::raw("  │  Poll: "),
            Span::raw(format!("{}s", state.poll_interval)),
            Span::raw("  │  Last: "),
            Span::raw(last_poll),
            Span::raw("  │  Uptime: "),
            Span::styled(uptime, Style::default().fg(ratatui::style::Color::Green)),
        ]),
    ];

    let header = Paragraph::new(header_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ratatui::style::Color::DarkGray))
            .title(Span::styled(
                " The Autonomous Software Factory ",
                Style::default()
                    .fg(ratatui::style::Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
    );

    f.render_widget(header, area);
}

fn render_pipelines(f: &mut ratatui::Frame, area: Rect, state: &DashboardState) {
    if state.pipelines.is_empty() {
        let msg = if state.last_poll.is_some() {
            vec![
                Line::raw(""),
                Line::styled(
                    "  No active pipelines",
                    Style::default().fg(ratatui::style::Color::DarkGray),
                ),
                Line::raw(""),
                Line::styled(
                    "  Waiting for issues labeled with the target label...",
                    Style::default().fg(ratatui::style::Color::DarkGray),
                ),
            ]
        } else {
            vec![
                Line::raw(""),
                Line::styled(
                    "  Starting up... waiting for first poll cycle",
                    Style::default()
                        .fg(ratatui::style::Color::Yellow)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]
        };

        let p = Paragraph::new(msg).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ratatui::style::Color::DarkGray))
                .title(" ACTIVE PIPELINES "),
        );
        f.render_widget(p, area);
        return;
    }

    // Each pipeline needs 4 lines (title + gauge + spacer + extra)
    let pipeline_height = 3u16;
    let total = state.pipelines.len() as u16;
    let constraints: Vec<Constraint> = (0..total)
        .map(|_| Constraint::Length(pipeline_height))
        .chain(std::iter::once(Constraint::Min(0)))
        .collect();

    let inner_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ratatui::style::Color::DarkGray))
        .title(" ACTIVE PIPELINES ");

    let inner_area = inner_block.inner(area);
    f.render_widget(inner_block, area);

    let pipeline_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner_area);

    for (i, pv) in state.pipelines.iter().enumerate() {
        render_single_pipeline(f, pipeline_chunks[i], pv);
    }
}

fn render_single_pipeline(f: &mut ratatui::Frame, area: Rect, pv: &PipelineView) {
    // Split into: label line (1) + gauge line (2)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(2)])
        .split(area);

    // Title line
    let (stage_color, stage_symbol) = match &pv.stage {
        Stage::Done => (ratatui::style::Color::Green, " ✓"),
        Stage::Failed(_) => (ratatui::style::Color::Red, " ✗"),
        _ => (ratatui::style::Color::Yellow, " ⚙"),
    };

    let title_line = Line::from(vec![
        Span::styled(
            format!("  #{:<5}", pv.issue_number),
            Style::default()
                .fg(ratatui::style::Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            truncate_str(&pv.title, 40),
            Style::default().fg(ratatui::style::Color::White),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{}", pv.stage),
            Style::default()
                .fg(stage_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(stage_symbol, Style::default().fg(stage_color)),
    ]);

    f.render_widget(Paragraph::new(title_line), chunks[0]);

    // Progress gauge
    let progress = if pv.is_failed {
        0.0
    } else {
        let idx = pv.stage.stage_index() as f64;
        let total = Stage::stage_count() as f64;
        idx / total
    };

    let gauge_color = match &pv.stage {
        Stage::Done => ratatui::style::Color::Green,
        Stage::Failed(_) => ratatui::style::Color::Red,
        _ => ratatui::style::Color::Yellow,
    };

    let label = format!(
        "  stage {}/{}",
        pv.stage.stage_index(),
        Stage::stage_count()
    );

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(gauge_color))
        .ratio(progress.clamp(0.0, 1.0))
        .label(label);

    f.render_widget(gauge, chunks[1]);
}

fn render_footer(f: &mut ratatui::Frame, area: Rect, state: &DashboardState) {
    let active = state
        .pipelines
        .iter()
        .filter(|p| !matches!(p.stage, Stage::Done | Stage::Failed(_)))
        .count();
    let done = state
        .pipelines
        .iter()
        .filter(|p| matches!(p.stage, Stage::Done))
        .count();
    let failed = state
        .pipelines
        .iter()
        .filter(|p| matches!(p.stage, Stage::Failed(_)))
        .count();

    let mut spans = vec![
        Span::raw("  "),
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(": quit  │  "),
        Span::styled(
            format!("{} active", active),
            Style::default().fg(ratatui::style::Color::Yellow),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{} done", done),
            Style::default().fg(ratatui::style::Color::Green),
        ),
    ];

    if failed > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("{} failed", failed),
            Style::default().fg(ratatui::style::Color::Red),
        ));
    }

    if let Some(ref err) = state.error_message {
        spans.push(Span::raw("  │  "));
        spans.push(Span::styled(
            truncate_str(err, 50),
            Style::default().fg(ratatui::style::Color::Red),
        ));
    }

    let footer = Paragraph::new(Line::from(spans))
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ratatui::style::Color::DarkGray)),
        );

    f.render_widget(footer, area);
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::Stage;

    #[test]
    fn test_stage_display() {
        assert_eq!(format!("{}", Stage::Ingest), "INGEST");
        assert_eq!(format!("{}", Stage::Understand), "UNDERSTAND");
        assert_eq!(format!("{}", Stage::Plan), "PLAN");
        assert_eq!(format!("{}", Stage::Implement), "IMPLEMENT");
        assert_eq!(format!("{}", Stage::Verify), "VERIFY");
        assert_eq!(format!("{}", Stage::Submit), "SUBMIT");
        assert_eq!(format!("{}", Stage::Watch), "WATCH");
        assert_eq!(format!("{}", Stage::Fix), "FIX");
        assert_eq!(format!("{}", Stage::Done), "DONE");
        assert_eq!(
            format!("{}", Stage::Failed("oops".to_string())),
            "FAILED: oops"
        );
    }

    #[test]
    fn test_stage_index() {
        assert_eq!(Stage::Ingest.stage_index(), 1);
        assert_eq!(Stage::Understand.stage_index(), 2);
        assert_eq!(Stage::Plan.stage_index(), 3);
        assert_eq!(Stage::Implement.stage_index(), 4);
        assert_eq!(Stage::Verify.stage_index(), 5);
        assert_eq!(Stage::Submit.stage_index(), 6);
        assert_eq!(Stage::Watch.stage_index(), 7);
        assert_eq!(Stage::Fix.stage_index(), 8);
        assert_eq!(Stage::Done.stage_index(), 9);
        assert_eq!(Stage::Failed("x".to_string()).stage_index(), 0);
    }

    #[test]
    fn test_stage_count() {
        assert_eq!(Stage::stage_count(), 9);
    }

    #[test]
    fn test_pipeline_view_from_pipeline() {
        let p = Pipeline {
            issue_number: 42,
            repo: "owner/repo".to_string(),
            stage: Stage::Implement,
            run_dir: std::path::PathBuf::from("/tmp/test"),
            worktree: std::path::PathBuf::from("/tmp/test/worktree"),
            pr_number: None,
            blocker_fingerprint: None,
            branch_name: "guild/issue-42".to_string(),
            issue_title: "Fix the auth bug".to_string(),
        };

        let view = PipelineView::from(&p);
        assert_eq!(view.issue_number, 42);
        assert_eq!(view.title, "Fix the auth bug");
        assert_eq!(view.stage, Stage::Implement);
        assert!(!view.is_failed);
    }

    #[test]
    fn test_pipeline_view_empty_title() {
        let p = Pipeline {
            issue_number: 7,
            repo: "owner/repo".to_string(),
            stage: Stage::Ingest,
            run_dir: std::path::PathBuf::from("/tmp/test"),
            worktree: std::path::PathBuf::from("/tmp/test/worktree"),
            pr_number: None,
            blocker_fingerprint: None,
            branch_name: "guild/issue-7".to_string(),
            issue_title: String::new(),
        };

        let view = PipelineView::from(&p);
        assert_eq!(view.title, "Issue #7");
    }

    #[test]
    fn test_dashboard_state_uptime() {
        let state = DashboardState::new("owner/repo".to_string(), "guild".to_string(), 30);
        let uptime = state.uptime_string();
        assert!(uptime.contains("0m"));
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world!!!", 5), "hell…");
    }
}

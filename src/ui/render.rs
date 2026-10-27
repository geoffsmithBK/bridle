//! Drawing. Everything here is a pure function of `App` (plus remembering
//! the pane height for half-screen scrolling).

use crate::events::ToolStatus;
use crate::ui::app::{App, Block};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as Frame_, Borders, Clear, Paragraph, Wrap};

/// Output lines shown for a running or collapsed command.
const TAIL_LINES: usize = 8;
/// Maximum visible input lines.
const INPUT_LINES: usize = 6;

pub fn draw(f: &mut Frame, app: &mut App) {
    let input_height = app.input.lines().len().clamp(1, INPUT_LINES) as u16 + 1;
    let [convo, input, status] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(input_height), Constraint::Length(1)])
            .areas(f.area());

    draw_conversation(f, app, convo);
    draw_input(f, app, input);
    draw_status(f, app, status);
    if app.pending.is_some() {
        draw_approval(f, app);
    }
    if app.wire_view.is_some() {
        draw_wire(f, app);
    }
}

fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

fn tail_lines(lines: &[String], out: &mut Vec<Line<'static>>) {
    let skip = lines.len().saturating_sub(TAIL_LINES);
    if skip > 0 {
        out.push(Line::styled(format!("│ … {skip} earlier lines"), dim()));
    }
    for l in &lines[skip..] {
        out.push(Line::styled(format!("│ {l}"), dim()));
    }
}

/// The whole conversation as styled lines (before wrapping).
pub fn conversation_lines(app: &App) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = vec![];
    for block in &app.blocks {
        match block {
            Block::User(text) => {
                for (j, l) in text.lines().enumerate() {
                    let prefix = if j == 0 { "› " } else { "  " };
                    out.push(Line::from(vec![
                        Span::styled(prefix, Style::default().fg(Color::Cyan).bold()),
                        Span::raw(l.to_string()).bold(),
                    ]));
                }
            }
            Block::Shell { command, live, output } => {
                out.push(Line::styled(format!("! {command}"), Style::default().fg(Color::Yellow).bold()));
                match output {
                    None => tail_lines(live, &mut out),
                    Some(o) => {
                        let lines: Vec<String> = o.lines().map(str::to_string).collect();
                        if app.show_tools {
                            lines.iter().for_each(|l| out.push(Line::styled(format!("│ {l}"), dim())));
                        } else {
                            tail_lines(&lines, &mut out);
                        }
                    }
                }
            }
            Block::Thinking { text, started, elapsed } => {
                // One line with a timer; the text itself only when toggled on.
                let label = match elapsed {
                    Some(d) => format!("thought for {}s", d.as_secs()),
                    None => format!("thinking… {}s", started.elapsed().as_secs()),
                };
                let arrow = if app.show_thinking { "▾" } else { "▸" };
                out.push(Line::styled(format!("{arrow} {label}"), dim()));
                if app.show_thinking {
                    for l in text.lines() {
                        out.push(Line::styled(l.to_string(), dim().add_modifier(Modifier::ITALIC)));
                    }
                }
            }
            Block::Assistant(text) => {
                let mut in_code = false;
                for l in text.lines() {
                    let fence = l.trim_start().starts_with("```");
                    if fence {
                        in_code = !in_code;
                    }
                    if in_code || fence {
                        out.push(Line::styled(l.to_string(), Style::default().fg(Color::Green)));
                    } else {
                        out.push(Line::raw(l.to_string()));
                    }
                }
            }
            Block::Tool { summary, status, live, result, .. } => {
                let (icon, color) = match status {
                    None => ("⏳", Color::Yellow),
                    Some(ToolStatus::Ok) => ("✓", Color::Green),
                    Some(ToolStatus::Error) => ("✗", Color::Red),
                    Some(ToolStatus::Denied) => ("⊘", Color::Magenta),
                    Some(ToolStatus::Cancelled) => ("⊘", Color::DarkGray),
                };
                let n = result.lines().count();
                let suffix = if status.is_some() && !app.show_tools && n > 0 {
                    format!("  ({n} lines)")
                } else {
                    String::new()
                };
                out.push(Line::from(vec![
                    Span::styled(format!("{icon} "), Style::default().fg(color)),
                    Span::raw(summary.clone()).bold(),
                    Span::styled(suffix, dim()),
                ]));
                match status {
                    None => tail_lines(live, &mut out),
                    Some(_) if app.show_tools => {
                        result.lines().for_each(|l| out.push(Line::styled(format!("│ {l}"), dim())));
                    }
                    // Collapsed: show only failures' first line, so errors stay visible.
                    Some(ToolStatus::Error) | Some(ToolStatus::Denied) => {
                        if let Some(l) = result.lines().next() {
                            out.push(Line::styled(format!("│ {l}"), Style::default().fg(color)));
                        }
                    }
                    Some(_) => {}
                }
            }
            Block::Info(text) => out.push(Line::styled(text.clone(), dim())),
            Block::Error(text) => {
                out.push(Line::styled(format!("✗ {text}"), Style::default().fg(Color::Red)))
            }
        }
        out.push(Line::raw(""));
    }
    out
}

fn draw_conversation(f: &mut Frame, app: &mut App, area: Rect) {
    app.view_height = area.height;
    let lines = conversation_lines(app);
    // Rows each line occupies once wrapped to the pane width.
    let heights: Vec<usize> = lines
        .iter()
        .map(|l| Paragraph::new(l.clone()).wrap(Wrap { trim: false }).line_count(area.width).max(1))
        .collect();
    let total: usize = heights.iter().sum();
    let height = area.height as usize;
    let max_back = total.saturating_sub(height);
    app.scroll_back = app.scroll_back.min(max_back);
    let top_row = total.saturating_sub(height + app.scroll_back);

    // Skip whole lines above the view; scroll the remainder within the first line.
    let mut first = 0;
    let mut row = 0;
    while first < lines.len() && row + heights[first] <= top_row {
        row += heights[first];
        first += 1;
    }
    let offset = (top_row - row) as u16;
    let visible: Vec<Line> = lines.into_iter().skip(first).collect();
    f.render_widget(Paragraph::new(visible).wrap(Wrap { trim: false }).scroll((offset, 0)), area);
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let title = if app.busy { " working… Ctrl-C interrupts " } else { "" };
    let frame = Frame_::default().borders(Borders::TOP).title(title).border_style(dim());
    let inner = frame.inner(area);
    f.render_widget(frame, area);

    let (row, col) = app.input.cursor_pos();
    let first = row.saturating_sub(INPUT_LINES - 1);
    let lines: Vec<Line> = app
        .input
        .lines()
        .into_iter()
        .enumerate()
        .skip(first)
        .map(|(i, l)| Line::raw(format!("{}{l}", if i == 0 { "> " } else { "  " })))
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
    if app.pending.is_none() && app.wire_view.is_none() {
        f.set_cursor_position(Position::new(inner.x + 2 + col as u16, inner.y + (row - first) as u16));
    }
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![];
    if app.online {
        let model = app.model.rsplit('/').next().unwrap_or(&app.model).trim_end_matches(".gguf");
        spans.push(Span::raw(format!(" {model}")));
    } else {
        spans.push(Span::styled(" server offline", Style::default().fg(Color::Red)));
    }
    spans.push(Span::styled(format!(" · ctx {}/{}", kilo(app.ctx_used), kilo(app.n_ctx)), dim()));
    if let Some(tps) = app.tokens_per_sec {
        spans.push(Span::styled(format!(" · {tps:.0} tok/s"), dim()));
    }
    if !app.activity.is_empty() {
        spans.push(Span::styled(format!(" · {}", app.activity), Style::default().fg(Color::Yellow)));
    }
    if app.ctrl_x {
        spans.push(Span::styled("  C-x-", Style::default().bold()));
    } else if app.quit_armed {
        spans.push(Span::styled("  Ctrl-C again to quit", Style::default().bold()));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn kilo(n: u64) -> String {
    if n >= 1000 { format!("{:.1}k", n as f64 / 1000.0) } else { n.to_string() }
}

fn centered(area: Rect, pct_w: u16, pct_h: u16) -> Rect {
    let w = area.width * pct_w / 100;
    let h = area.height * pct_h / 100;
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn draw_approval(f: &mut Frame, app: &App) {
    let Some(p) = &app.pending else { return };
    let area = centered(f.area(), 90, 80);
    f.render_widget(Clear, area);

    let mut title = vec![Span::raw(format!(" {} ", p.request.title)).bold()];
    if !p.request.subject.is_empty() {
        title.push(Span::raw(format!("─ {} ", p.request.subject)));
    }
    if p.request.overwrite {
        title.push(Span::styled(" OVERWRITE ", Style::default().fg(Color::Yellow).bold()));
    }
    if p.request.outside_project {
        title
            .push(Span::styled(" OUTSIDE PROJECT ", Style::default().fg(Color::White).bg(Color::Red).bold()));
    }
    let frame = Frame_::default().borders(Borders::ALL).title(Line::from(title));
    let inner = frame.inner(area);
    f.render_widget(frame, area);
    let [body, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);

    let is_diff = p.request.body.starts_with("--- ");
    let lines: Vec<Line> = p
        .request
        .body
        .lines()
        .map(|l| {
            let style = if !is_diff {
                Style::default()
            } else if l.starts_with("@@") {
                Style::default().fg(Color::Cyan)
            } else if l.starts_with('+') {
                Style::default().fg(Color::Green)
            } else if l.starts_with('-') {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            Line::styled(l.to_string(), style)
        })
        .collect();
    f.render_widget(Paragraph::new(lines).scroll((p.scroll, 0)), body);

    match &p.reason {
        None => f.render_widget(
            Paragraph::new(" [y] approve   [n] deny   [r] deny with reason   (j/k scroll)").bold(),
            footer,
        ),
        Some(reason) => {
            let text = reason.text();
            f.render_widget(Paragraph::new(format!(" reason: {text}")), footer);
            let col = reason.cursor_pos().1 as u16;
            f.set_cursor_position(Position::new(footer.x + 9 + col, footer.y));
        }
    }
}

fn draw_wire(f: &mut Frame, app: &App) {
    let area = centered(f.area(), 96, 92);
    f.render_widget(Clear, area);
    let title = format!(" wire log — {} requests · j/k PgUp/PgDn scroll · Esc closes ", app.wire.len());
    let mut lines: Vec<Line> = vec![];
    for (i, w) in app.wire.iter().enumerate() {
        lines.push(Line::styled(format!("─── exchange {} ───", i + 1), Style::default().fg(Color::Cyan)));
        lines.extend(w.lines().map(|l| Line::raw(l.to_string())));
    }
    let scroll = app.wire_view.unwrap_or(0);
    f.render_widget(
        Paragraph::new(lines).scroll((scroll, 0)).block(Frame_::default().borders(Borders::ALL).title(title)),
        area,
    );
}

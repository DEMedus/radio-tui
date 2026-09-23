//! Draws the screens: station list, EQ, edit, import, and text prompts.

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::app::{App, Mode};
use crate::visualizer::BAR_COUNT;

pub fn ui(frame: &mut Frame, app: &mut App) {
    match app.mode {
        Mode::Help => draw_help(frame),
        Mode::Edit => draw_edit(frame, app),
        Mode::NowPlaying => draw_now_playing(frame, app),
        Mode::ImportPick => draw_import_pick(frame, app),
        Mode::AddName { .. }
        | Mode::AddUrl { .. }
        | Mode::EditUrl { .. }
        | Mode::ImportUrl { .. } => {
            if matches!(app.mode, Mode::ImportUrl { .. } | Mode::EditUrl { .. }) {
                draw_edit(frame, app);
            } else {
                draw_main(frame, app);
            }
            draw_input(frame, app);
        }
        Mode::Normal => draw_main(frame, app),
    }
}

fn draw_main(frame: &mut Frame, app: &mut App) {
    let width = frame.area().width;
    let footer_h = legend_footer_height(KEY_LEGEND, width);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(6),
            Constraint::Length(footer_h),
        ])
        .split(frame.area());

    let title = Paragraph::new(" radio-tui • Internet Radio ")
        .style(Style::default().fg(Color::Green))
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    draw_station_list(frame, app, chunks[1], " Saved Stations ");
    draw_visualizer(frame, app, chunks[2]);
    draw_footer(frame, app, chunks[3]);
}

fn draw_edit(frame: &mut Frame, app: &mut App) {
    let legend = "j/k move • d del • u url • J/K order • * fav • i import • Enter play • Esc back";
    let footer_h = legend_footer_height(legend, frame.area().width);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(footer_h)])
        .split(frame.area());

    draw_station_list(frame, app, chunks[0], " Edit stations ");
    draw_mode_footer(frame, chunks[1], &app.status, legend);
}

fn draw_station_list(frame: &mut Frame, app: &mut App, area: Rect, title: &str) {
    if app.stations.is_empty() {
        let empty = Paragraph::new("No stations yet. Press a to add one.")
            .style(Style::default().fg(Color::DarkGray))
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            );
        frame.render_widget(empty, area);
        return;
    }

    let playing = app.playing_index();
    let items: Vec<ListItem> = app
        .stations
        .iter()
        .enumerate()
        .map(|(i, station)| {
            let marker = if playing == Some(i) { "♪ " } else { "" };
            let star = if station.favorite { "★ " } else { "" };
            ListItem::new(Line::from(vec![
                Span::styled(star, Style::default().fg(Color::Yellow)),
                Span::styled(marker, Style::default().fg(Color::Green)),
                Span::styled(station.name.as_str(), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(station.url.as_str(), Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_visualizer(frame: &mut Frame, app: &App, area: Rect) {
    let widget = if app.is_playing() {
        let name = app
            .playing_index()
            .and_then(|i| app.stations.get(i))
            .map(|s| s.name.as_str())
            .unwrap_or("stream");
        let (top, bottom) = eq_lines(app.eq_bars());
        Paragraph::new(vec![
            Line::from(vec![Span::styled(
                "LIVE",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )]),
            top,
            bottom,
            Line::from(Span::styled(name, Style::default().fg(Color::Gray))),
        ])
        .block(Block::default().title(" EQ ").borders(Borders::ALL))
    } else {
        Paragraph::new("Stopped — select a station and press Space or Enter")
            .style(Style::default().fg(Color::DarkGray))
            .wrap(Wrap { trim: true })
            .block(Block::default().title(" EQ ").borders(Borders::ALL))
    };
    frame.render_widget(widget, area);
}

fn eq_lines(levels: &[f32]) -> (Line<'static>, Line<'static>) {
    const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let mut top = Vec::new();
    let mut bottom = Vec::new();
    for (i, &level) in levels.iter().enumerate() {
        let steps = (level.clamp(0.0, 1.0) * 16.0).round() as u8;
        let bottom_n = steps.min(8) as usize;
        let top_n = steps.saturating_sub(8) as usize;
        let color = if level > 0.92 {
            Color::Red
        } else if level > 0.78 {
            Color::Yellow
        } else {
            Color::Green
        };
        if i > 0 {
            top.push(Span::raw(" "));
            bottom.push(Span::raw(" "));
        }
        top.push(Span::styled(
            BLOCKS[top_n].to_string(),
            Style::default().fg(color),
        ));
        bottom.push(Span::styled(
            BLOCKS[bottom_n].to_string(),
            Style::default().fg(color),
        ));
    }
    (Line::from(top), Line::from(bottom))
}

const KEY_LEGEND: &str =
    "j/k move • Enter play • Space stop • +/- vol • * fav • a add • e edit • f now • ? help • q quit";

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let status = format!("Volume {:.0}", app.volume) + "%  •  " + app.status.as_str();
    draw_mode_footer(frame, area, &status, KEY_LEGEND);
}

fn draw_now_playing(frame: &mut Frame, app: &App) {
    let energy = app.eq_energy();
    let border = if energy > 0.68 {
        Color::Yellow
    } else if energy > 0.32 {
        Color::Green
    } else {
        Color::DarkGray
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(border))
        .title(" ON AIR ")
        .title_style(
            Style::default()
                .fg(if (app.eq_tick() / 8).is_multiple_of(2) {
                    Color::Red
                } else {
                    Color::DarkGray
                })
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(frame.area());
    frame.render_widget(block, frame.area());

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(inner);

    let station = center_text(app.playing_station_name(), chunks[1].width as usize);
    frame.render_widget(
        Paragraph::new(station).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        chunks[1],
    );

    if let Some(icy) = app.icy_name() {
        if icy != app.playing_station_name() {
            frame.render_widget(
                Paragraph::new(center_text(icy, chunks[2].width as usize))
                    .style(Style::default().fg(Color::DarkGray)),
                chunks[2],
            );
        }
    }

    let track = app
        .track_title()
        .unwrap_or("Live stream — no track metadata");
    let track = marquee(track, chunks[3].width as usize, app.eq_tick());
    frame.render_widget(
        Paragraph::new(format!("♪  {track}")).style(Style::default().fg(Color::Cyan)),
        chunks[3],
    );

    draw_mirror_eq(frame, chunks[5], app.eq_bars(), energy);

    let hint = format!("volume {:.0}%   any key returns to the list", app.volume);
    frame.render_widget(
        Paragraph::new(hint)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center),
        chunks[6],
    );
}

fn draw_mirror_eq(frame: &mut Frame, area: Rect, bars: &[f32], energy: f32) {
    if area.width < 8 || area.height < 4 {
        return;
    }
    let height = area.height as usize;
    let width = area.width as usize;
    let mid = height / 2;
    let count = bars.len().clamp(1, BAR_COUNT);
    let gap = 1usize;
    let bar_w = ((width.saturating_sub(gap * count.saturating_sub(1))) / count).max(1);
    let used = count * bar_w + count.saturating_sub(1) * gap;
    let pad = width.saturating_sub(used) / 2;

    let mut lines = Vec::with_capacity(height);
    for y in 0..height {
        let mut spans: Vec<Span> = Vec::new();
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad)));
        }
        for (i, &level) in bars.iter().take(count).enumerate() {
            if i > 0 {
                spans.push(Span::raw(" ".repeat(gap)));
            }
            let upper = y < mid;
            let dist = if upper { mid - y } else { y - mid };
            let max_dist = if upper {
                mid.max(1)
            } else {
                (height - mid).max(1)
            };
            let filled = (level.clamp(0.0, 1.0) * max_dist as f32).round() as usize;
            let on_horizon = dist == 0;
            let glyph = if on_horizon {
                if energy > 0.08 {
                    '─'
                } else {
                    '·'
                }
            } else if dist <= filled {
                '█'
            } else if dist == filled + 1 && level > 0.08 {
                if upper {
                    '▄'
                } else {
                    '▀'
                }
            } else {
                ' '
            };
            let color = if on_horizon || !upper {
                Color::DarkGray
            } else if level > 0.92 {
                Color::Red
            } else if level > 0.78 {
                Color::Yellow
            } else {
                Color::Green
            };
            spans.push(Span::styled(
                glyph.to_string().repeat(bar_w),
                Style::default().fg(color),
            ));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn center_text(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() >= width {
        return chars.into_iter().take(width).collect();
    }
    let pad = (width - chars.len()) / 2;
    format!("{}{text}", " ".repeat(pad))
}

fn marquee(text: &str, width: usize, tick: u32) -> String {
    if width == 0 {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_string();
    }
    let padded: Vec<char> = format!("   {text}   •  ").chars().collect();
    let start = (tick as usize / 2) % padded.len();
    padded.iter().cycle().skip(start).take(width).collect()
}

fn draw_import_pick(frame: &mut Frame, app: &mut App) {
    let Some(import) = app.import.as_mut() else {
        return;
    };
    let legend = "Space toggle • a all • n none • Enter Do it! • Esc cancel";
    let footer_h = legend_footer_height(legend, frame.area().width).max(4);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(footer_h)])
        .split(frame.area());

    let items: Vec<ListItem> = import
        .stations
        .iter()
        .enumerate()
        .map(|(i, station)| {
            let (mark, style) = if import.already.get(i).copied().unwrap_or(false) {
                ("have", Style::default().fg(Color::DarkGray))
            } else if import.chosen.get(i).copied().unwrap_or(false) {
                (" +  ", Style::default().fg(Color::Green))
            } else {
                ("    ", Style::default().fg(Color::White))
            };
            ListItem::new(Line::from(vec![
                Span::styled(mark, style),
                Span::raw("  "),
                Span::styled(station.name.as_str(), style),
                Span::raw("  "),
                Span::styled(station.url.as_str(), Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" Import from {} ", import.source))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(list, chunks[0], &mut import.list_state);

    let picked = import.chosen.iter().filter(|pick| **pick).count();
    let already = import.already.iter().filter(|have| **have).count();
    let summary = format!(
        "{picked} selected • {already} already saved\n{}",
        app.status
    );
    draw_mode_footer(frame, chunks[1], &summary, legend);
}

/// Border plus one status line, then one line per wrapped key row.
fn legend_footer_height(legend: &str, width: u16) -> u16 {
    let inner = width.saturating_sub(2) as usize;
    let rows = wrap_legend(legend, inner).len().clamp(1, 4);
    3 + rows as u16
}

fn draw_mode_footer(frame: &mut Frame, area: Rect, status: &str, legend: &str) {
    let inner = area.width.saturating_sub(2) as usize;
    let keys = wrap_legend(legend, inner).join("\n");
    let footer = Paragraph::new(format!("{status}\n{keys}"))
        .style(Style::default().fg(Color::Cyan))
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, area);
}

/// Break the legend on the ` • ` separators so a key and its word stay together.
fn wrap_legend(legend: &str, width: usize) -> Vec<String> {
    if width < 4 {
        return vec![legend.to_string()];
    }
    let tokens: Vec<&str> = legend.split(" • ").collect();
    let mut lines = Vec::new();
    let mut current = String::new();
    for token in tokens {
        let extra = if current.is_empty() { 0 } else { 3 };
        if !current.is_empty() && current.len() + extra + token.len() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str(" • ");
        }
        current.push_str(token);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn draw_help(frame: &mut Frame) {
    let help = Paragraph::new(
        "radio-tui controls\n\n\
         j / k  or  ↑ / ↓     Move selection\n\
         Enter                Play selected station\n\
         Space                Stop / play\n\
         + / -                Volume up / down\n\
         a                    Add a station (name, then URL)\n\
         e                    Edit list (delete / reorder / import)\n\
         *                    Favorite selected station (pops to the top)\n\
         i                    Import stations from a remote JSON list\n\
         u                    Edit the selected station URL (edit mode)\n\
         d                    Delete selected station (edit mode)\n\
         J / K                Move station down / up (edit mode)\n\
         ?                    Toggle this help\n\
         f                    Now-playing screen (also after 1 min idle)\n\
         q                    Quit\n\
         Esc                  Leave overlay / quit from the list\n\
         Ctrl+C               Quit immediately\n\n\
         After a minute of playback with no keys, the list hides and a\n\
         now-playing screen shows the station, track (if the stream sends\n\
         ICY metadata), and a full-screen EQ. Press f to open it anytime\n\
         while playing. Any key returns to the list.\n\n\
         Streams are played with mpv. Config is saved to\n\
         ~/.config/radio-tui/stations.json\n\n\
         Press any key to return.",
    )
    .style(Style::default().fg(Color::Yellow))
    .wrap(Wrap { trim: false })
    .block(Block::default().title(" Help ").borders(Borders::ALL));
    frame.render_widget(help, frame.area());
}

fn cursor_line(buffer: &str, cursor: usize) -> String {
    let cursor = cursor.min(buffer.chars().count());
    let byte = buffer
        .char_indices()
        .nth(cursor)
        .map(|(index, _)| index)
        .unwrap_or(buffer.len());
    let (head, tail) = buffer.split_at(byte);
    format!("{head}█{tail}")
}

fn draw_input(frame: &mut Frame, app: &App) {
    let Some((label, buffer, cursor)) = app.input_field() else {
        return;
    };
    let area = centered_rect(70, 9, frame.area());
    frame.render_widget(Clear, area);
    let hint = match app.mode {
        Mode::ImportUrl { .. } => "Enter fetch  •  Esc cancel",
        Mode::EditUrl { .. } => "←/→ move  •  Enter save  •  Esc cancel",
        _ => "Enter confirm  •  Esc cancel",
    };
    let title = match app.mode {
        Mode::EditUrl { .. } => " Edit URL ",
        Mode::ImportUrl { .. } => " Import list ",
        _ => " Add station ",
    };
    let body = format!("{label}\n\n{}\n\n{hint}", cursor_line(buffer, cursor));
    let widget = Paragraph::new(body)
        .style(Style::default().fg(Color::Cyan))
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        );
    frame.render_widget(widget, area);
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::{wrap_legend, KEY_LEGEND};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn wrap_legend_keeps_each_key_with_its_word() {
        let lines = wrap_legend(KEY_LEGEND, 40);
        let joined = lines.join("\n");
        assert!(joined.contains("j/k move"));
        assert!(joined.contains("* fav"));
        assert!(joined.contains("q quit"));
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| line.len() <= 40), "{lines:?}");
    }

    fn render(width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = crate::app::App::fixture();
        terminal.draw(|frame| super::ui(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut lines = Vec::new();
        for y in 0..height {
            let row: String = (0..width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect();
            lines.push(row.trim_end().to_string());
        }
        lines.join("\n")
    }

    #[test]
    fn narrow_screen_shows_every_key_without_clipping() {
        let screen = render(50, 28);
        for token in ["j/k move", "* fav", "f now", "? help", "q quit"] {
            assert!(screen.contains(token), "missing {token}:\n{screen}");
        }
    }

    #[test]
    fn wide_screen_fits_the_legend_on_one_line() {
        let screen = render(120, 24);
        assert!(screen.contains(KEY_LEGEND), "{screen}");
    }
}

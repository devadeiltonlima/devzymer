use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ratatui::crossterm::cursor::{Hide, MoveToColumn, MoveUp, Show};
use ratatui::crossterm::queue;
use ratatui::crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor,
};
use ratatui::crossterm::terminal::{self, Clear, ClearType};
use ratatui::style::{Color as RColor, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::banner_art::{ART, ROWS, WIDTH};

pub const TAGLINE: &str = "Less Tokens. More Engineering.";
pub const BANNER_WIDTH: usize = WIDTH;
pub const BANNER_ROWS: usize = ROWS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

const STOPS: [Rgb; 3] = [Rgb(0, 245, 160), Rgb(0, 217, 245), Rgb(124, 77, 255)];
const BACKGROUND: Rgb = Rgb(13, 17, 23);
const MUTED: Rgb = Rgb(139, 148, 158);
const GOOD: Rgb = Rgb(63, 185, 80);
const BAD: Rgb = Rgb(248, 81, 73);
const WARN: Rgb = Rgb(210, 153, 34);
const WHITE: Rgb = Rgb(230, 237, 243);

const SPINNER_UNICODE: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_ASCII: [&str; 4] = ["|", "/", "-", "\\"];
const BAR_CELLS: usize = 28;

#[derive(Debug, Clone, Copy)]
pub struct Caps {
    pub color: bool,
    pub truecolor: bool,
    pub unicode: bool,
    pub animate: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Ok,
    Fail,
    Warn,
    Info,
}

fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

pub fn detect() -> Caps {
    let tty = io::stdout().is_terminal();
    let forced = env_flag("DEVZYMER_FORCE_FANCY");
    let dumb = std::env::var("TERM").is_ok_and(|term| term == "dumb");
    let color = (tty || forced) && !env_flag("NO_COLOR") && !dumb;
    let truecolor = color
        && (cfg!(windows)
            || env_flag("WT_SESSION")
            || std::env::var("COLORTERM")
                .is_ok_and(|value| value.contains("truecolor") || value.contains("24bit")));
    let unicode = if cfg!(windows) {
        forced
            || env_flag("WT_SESSION")
            || env_flag("TERM_PROGRAM")
            || std::env::var("ConEmuANSI").is_ok_and(|value| value == "ON")
    } else {
        true
    };
    let animate = tty && color && !env_flag("DEVZYMER_NO_ANIM") && !env_flag("CI");
    Caps {
        color,
        truecolor,
        unicode,
        animate,
    }
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (f32::from(a) + (f32::from(b) - f32::from(a)) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    Rgb(lerp(a.0, b.0, t), lerp(a.1, b.1, t), lerp(a.2, b.2, t))
}

pub fn gradient(t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0) * 2.0;
    if t <= 1.0 {
        mix(STOPS[0], STOPS[1], t)
    } else {
        mix(STOPS[1], STOPS[2], t - 1.0)
    }
}

pub fn glyph_color(column: usize, glyph: char, shimmer: Option<f32>) -> Rgb {
    let mut color = gradient(column as f32 / (WIDTH - 1) as f32);
    if glyph == '▒' {
        color = mix(BACKGROUND, color, 0.42);
    }
    if let Some(center) = shimmer {
        let distance = (column as f32 - center).abs();
        let boost = (1.0 - distance / 9.0).max(0.0);
        if boost > 0.0 {
            color = mix(color, WHITE, boost * 0.7);
        }
    }
    color
}

pub fn shimmer_center(tick: u64) -> f32 {
    ((tick % 90) * 2) as f32 - 18.0
}

fn cube(rgb: Rgb) -> u8 {
    let level = |value: u8| ((u16::from(value) * 5 + 127) / 255) as u8;
    16 + 36 * level(rgb.0) + 6 * level(rgb.1) + level(rgb.2)
}

fn term_color(rgb: Rgb, caps: &Caps) -> Color {
    if caps.truecolor {
        Color::Rgb {
            r: rgb.0,
            g: rgb.1,
            b: rgb.2,
        }
    } else {
        Color::AnsiValue(cube(rgb))
    }
}

pub fn ratatui_color(rgb: Rgb, caps: &Caps) -> RColor {
    if caps.truecolor {
        RColor::Rgb(rgb.0, rgb.1, rgb.2)
    } else {
        RColor::Indexed(cube(rgb))
    }
}

pub fn banner_lines(caps: &Caps, tick: u64) -> Vec<Line<'static>> {
    let shimmer = Some(shimmer_center(tick));
    let mut lines: Vec<Line<'static>> = ART
        .iter()
        .map(|row| {
            let spans: Vec<Span<'static>> = row
                .chars()
                .enumerate()
                .map(|(column, glyph)| {
                    if glyph == ' ' || !caps.color {
                        Span::raw(glyph.to_string())
                    } else {
                        let color = ratatui_color(glyph_color(column, glyph, shimmer), caps);
                        Span::styled(glyph.to_string(), Style::default().fg(color))
                    }
                })
                .collect();
            Line::from(spans)
        })
        .collect();
    let tagline_color = ratatui_color(MUTED, caps);
    lines.push(Line::from(Span::styled(
        TAGLINE.to_string(),
        Style::default()
            .fg(tagline_color)
            .add_modifier(Modifier::ITALIC),
    )));
    lines
}

fn queue_row<W: Write>(
    out: &mut W,
    caps: &Caps,
    row: &str,
    shimmer: Option<f32>,
) -> io::Result<()> {
    let mut last: Option<Color> = None;
    for (column, glyph) in row.chars().enumerate() {
        if glyph == ' ' {
            queue!(out, Print(' '))?;
            continue;
        }
        let color = term_color(glyph_color(column, glyph, shimmer), caps);
        if last != Some(color) {
            queue!(out, SetForegroundColor(color))?;
            last = Some(color);
        }
        queue!(out, Print(glyph))?;
    }
    queue!(out, ResetColor, Print("\r\n"))
}

fn queue_tagline<W: Write>(out: &mut W, caps: &Caps, version: &str) -> io::Result<()> {
    let right = format!("v{version}");
    let gap = WIDTH.saturating_sub(TAGLINE.chars().count() + right.chars().count() + 1);
    if caps.color {
        queue!(
            out,
            Print(" "),
            SetForegroundColor(term_color(gradient(0.0), caps)),
            SetAttribute(Attribute::Italic),
            Print(TAGLINE),
            SetAttribute(Attribute::Reset),
            Print(" ".repeat(gap)),
            SetForegroundColor(term_color(MUTED, caps)),
            Print(right),
            ResetColor,
            Print("\r\n\r\n")
        )
    } else {
        queue!(
            out,
            Print(" "),
            Print(TAGLINE),
            Print(" ".repeat(gap)),
            Print(right),
            Print("\r\n\r\n")
        )
    }
}

fn queue_compact<W: Write>(out: &mut W, caps: &Caps, version: &str) -> io::Result<()> {
    queue!(out, Print("\r\n  "))?;
    let word = "DEVZYMER";
    for (index, letter) in word.chars().enumerate() {
        if caps.color {
            let color = term_color(gradient(index as f32 / (word.len() - 1) as f32), caps);
            queue!(
                out,
                SetForegroundColor(color),
                SetAttribute(Attribute::Bold),
                Print(letter)
            )?;
        } else {
            queue!(out, Print(letter))?;
        }
    }
    queue!(out, SetAttribute(Attribute::Reset), ResetColor)?;
    if caps.color {
        queue!(
            out,
            SetForegroundColor(term_color(MUTED, caps)),
            Print(format!("  v{version}")),
            ResetColor
        )?;
    } else {
        queue!(out, Print(format!("  v{version}")))?;
    }
    queue!(out, Print(format!("\r\n  {TAGLINE}\r\n\r\n")))
}

pub fn print_banner(caps: &Caps, animated: bool, version: &str) -> io::Result<()> {
    let mut out = io::stdout().lock();
    let width = terminal::size()
        .map(|(columns, _)| usize::from(columns))
        .unwrap_or(WIDTH + 2);
    if width < WIDTH + 2 {
        queue_compact(&mut out, caps, version)?;
        return out.flush();
    }
    queue!(out, Print("\r\n"))?;
    if !caps.color {
        for row in ART {
            queue!(out, Print(row.trim_end()), Print("\r\n"))?;
        }
    } else if animated && caps.animate {
        queue!(out, Hide)?;
        for row in ART {
            queue_row(&mut out, caps, row, None)?;
            out.flush()?;
            thread::sleep(Duration::from_millis(60));
        }
        for frame in 0..16u32 {
            queue!(out, MoveUp(ROWS as u16))?;
            let center = -10.0 + frame as f32 * 8.0;
            for row in ART {
                queue_row(&mut out, caps, row, Some(center))?;
            }
            out.flush()?;
            thread::sleep(Duration::from_millis(50));
        }
        queue!(out, MoveUp(ROWS as u16))?;
        for row in ART {
            queue_row(&mut out, caps, row, None)?;
        }
        queue!(out, Show)?;
    } else {
        for row in ART {
            queue_row(&mut out, caps, row, None)?;
        }
    }
    queue_tagline(&mut out, caps, version)?;
    out.flush()
}

fn mark_parts(caps: &Caps, mark: Mark) -> (&'static str, Rgb) {
    match (mark, caps.unicode) {
        (Mark::Ok, true) => ("✔", GOOD),
        (Mark::Ok, false) => ("+", GOOD),
        (Mark::Fail, true) => ("✖", BAD),
        (Mark::Fail, false) => ("x", BAD),
        (Mark::Warn, true) => ("▲", WARN),
        (Mark::Warn, false) => ("!", WARN),
        (Mark::Info, true) => ("•", MUTED),
        (Mark::Info, false) => ("-", MUTED),
    }
}

fn padded(label: &str) -> String {
    if label.chars().count() >= 34 {
        format!("{label}  ")
    } else {
        format!("{label:<34}")
    }
}

pub fn line(caps: &Caps, mark: Mark, label: &str, detail: &str) -> io::Result<()> {
    let mut out = io::stdout().lock();
    let (symbol, color) = mark_parts(caps, mark);
    if caps.color {
        queue!(
            out,
            MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            Print("  "),
            SetForegroundColor(term_color(color, caps)),
            Print(symbol),
            ResetColor,
            Print(format!(" {}", padded(label))),
            SetForegroundColor(term_color(MUTED, caps)),
            Print(detail),
            ResetColor,
            Print("\r\n")
        )?;
    } else {
        queue!(
            out,
            Print(format!("  {symbol} {}{detail}\r\n", padded(label)))
        )?;
    }
    out.flush()
}

pub fn heading(caps: &Caps, text: &str) -> io::Result<()> {
    let mut out = io::stdout().lock();
    if caps.color {
        queue!(
            out,
            Print("  "),
            SetForegroundColor(term_color(gradient(0.5), caps)),
            SetAttribute(Attribute::Bold),
            Print(text),
            SetAttribute(Attribute::Reset),
            ResetColor,
            Print("\r\n\r\n")
        )?;
    } else {
        queue!(out, Print(format!("  {text}\r\n\r\n")))?;
    }
    out.flush()
}

fn spin(caps: Caps, label: String, stop: Arc<AtomicBool>) {
    let frames: &[&str] = if caps.unicode {
        &SPINNER_UNICODE
    } else {
        &SPINNER_ASCII
    };
    let started = Instant::now();
    let mut tick = 0usize;
    while !stop.load(Ordering::SeqCst) {
        {
            let mut out = io::stdout().lock();
            let phase = (tick % 40) as f32 / 39.0;
            let _ = queue!(
                out,
                MoveToColumn(0),
                Clear(ClearType::CurrentLine),
                Print("  "),
                SetForegroundColor(term_color(gradient(phase), &caps)),
                Print(frames[tick % frames.len()]),
                ResetColor,
                Print(format!(" {label}")),
                SetForegroundColor(term_color(MUTED, &caps)),
                Print(format!("  {:.1}s", started.elapsed().as_secs_f32())),
                ResetColor
            );
            let _ = out.flush();
        }
        tick += 1;
        thread::sleep(Duration::from_millis(80));
    }
    let mut out = io::stdout().lock();
    let _ = queue!(out, MoveToColumn(0), Clear(ClearType::CurrentLine));
    let _ = out.flush();
}

pub struct Task {
    caps: Caps,
    label: String,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    done: bool,
}

impl Task {
    pub fn start(caps: &Caps, label: &str) -> Task {
        let stop = Arc::new(AtomicBool::new(false));
        let handle = if caps.animate {
            let flag = stop.clone();
            let copy = *caps;
            let text = label.to_string();
            Some(thread::spawn(move || spin(copy, text, flag)))
        } else {
            None
        };
        Task {
            caps: *caps,
            label: label.to_string(),
            stop,
            handle,
            done: false,
        }
    }

    pub fn finish(mut self, detail: &str) {
        self.end(Mark::Ok, detail);
    }

    pub fn fail(mut self, detail: &str) {
        self.end(Mark::Fail, detail);
    }

    fn end(&mut self, mark: Mark, detail: &str) {
        self.halt();
        let _ = line(&self.caps, mark, &self.label, detail);
        self.done = true;
    }

    fn halt(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if !self.done {
            self.halt();
        }
    }
}

pub fn human_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB)
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

pub struct Bar {
    caps: Caps,
    label: String,
    total: Option<u64>,
    done: u64,
    last_draw: Instant,
}

impl Bar {
    pub fn new(caps: &Caps, label: &str, total: Option<u64>) -> Bar {
        Bar {
            caps: *caps,
            label: label.to_string(),
            total,
            done: 0,
            last_draw: Instant::now() - Duration::from_secs(1),
        }
    }

    pub fn advance(&mut self, bytes: u64) {
        self.done += bytes;
        if !self.caps.animate || self.last_draw.elapsed() < Duration::from_millis(60) {
            return;
        }
        self.last_draw = Instant::now();
        self.draw();
    }

    fn draw(&self) {
        let caps = &self.caps;
        let (full, empty) = if caps.unicode {
            ('█', '░')
        } else {
            ('#', '-')
        };
        let ratio = self
            .total
            .filter(|total| *total > 0)
            .map(|total| (self.done as f64 / total as f64).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        let filled = (ratio * BAR_CELLS as f64).round() as usize;
        let mut out = io::stdout().lock();
        let _ = queue!(
            out,
            MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            Print(format!("  {:<30} ", self.label))
        );
        for cell in 0..BAR_CELLS {
            if cell < filled {
                let color = term_color(gradient(cell as f32 / (BAR_CELLS - 1) as f32), caps);
                let _ = queue!(out, SetForegroundColor(color), Print(full));
            } else {
                let _ = queue!(
                    out,
                    SetForegroundColor(term_color(MUTED, caps)),
                    Print(empty)
                );
            }
        }
        let detail = match self.total {
            Some(total) if total > 0 => format!(
                " {:>3.0}%  {} / {}",
                ratio * 100.0,
                human_bytes(self.done),
                human_bytes(total)
            ),
            _ => format!("  {}", human_bytes(self.done)),
        };
        let _ = queue!(
            out,
            ResetColor,
            SetForegroundColor(term_color(MUTED, caps)),
            Print(detail),
            ResetColor
        );
        let _ = out.flush();
    }

    pub fn finish(self, detail: &str) {
        let _ = line(&self.caps, Mark::Ok, &self.label, detail);
    }

    pub fn fail(self, detail: &str) {
        let _ = line(&self.caps, Mark::Fail, &self.label, detail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banner_has_the_expected_shape() {
        assert_eq!(ART.len(), ROWS);
        assert!(ART.iter().all(|row| row.chars().count() == WIDTH));
        assert!(ART[0].contains('█'));
        assert!(ART.iter().any(|row| row.contains('▒')));
    }

    #[test]
    fn the_gradient_runs_from_the_first_to_the_last_stop() {
        assert_eq!(gradient(0.0), STOPS[0]);
        assert_eq!(gradient(0.5), STOPS[1]);
        assert_eq!(gradient(1.0), STOPS[2]);
        assert_eq!(gradient(-3.0), STOPS[0]);
        assert_eq!(gradient(9.0), STOPS[2]);
    }

    #[test]
    fn a_shadow_glyph_is_darker_than_a_solid_glyph() {
        let solid = glyph_color(40, '█', None);
        let shadow = glyph_color(40, '▒', None);
        let sum = |c: Rgb| u32::from(c.0) + u32::from(c.1) + u32::from(c.2);
        assert!(sum(shadow) < sum(solid));
    }

    #[test]
    fn the_shimmer_brightens_only_near_its_center() {
        let near = glyph_color(50, '█', Some(50.0));
        let far = glyph_color(50, '█', Some(100.0));
        let plain = glyph_color(50, '█', None);
        let sum = |c: Rgb| u32::from(c.0) + u32::from(c.1) + u32::from(c.2);
        assert!(sum(near) > sum(plain));
        assert_eq!(far, plain);
    }

    #[test]
    fn a_256_color_fallback_stays_inside_the_color_cube() {
        for rgb in [
            Rgb(0, 0, 0),
            Rgb(255, 255, 255),
            Rgb(0, 245, 160),
            Rgb(124, 77, 255),
        ] {
            let index = cube(rgb);
            assert!((16..=231).contains(&index), "{index}");
        }
    }

    #[test]
    fn a_long_label_keeps_a_gap_before_its_detail() {
        assert_eq!(padded("curto").chars().count(), 34);
        let long = "devzymer-arm64-unknown-linux-gnu.tar.gz";
        assert!(padded(long).ends_with("  "));
        assert_eq!(padded(long).chars().count(), long.chars().count() + 2);
    }

    #[test]
    fn byte_counts_are_shown_in_a_readable_unit() {
        assert_eq!(human_bytes(2048), "2 KB");
        assert_eq!(human_bytes(6 * 1024 * 1024), "6.0 MB");
    }

    #[test]
    fn a_task_without_animation_never_spawns_a_thread() {
        let caps = Caps {
            color: false,
            truecolor: false,
            unicode: false,
            animate: false,
        };
        let task = Task::start(&caps, "etapa");
        assert!(task.handle.is_none());
        drop(task);
    }
}

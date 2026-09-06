use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;
use std::time::{SystemTime, UNIX_EPOCH};

/// 一行高度的动画状态栏：播放时显示跳动的彩虹均衡器，暂停/停止时静止。
pub struct StatusBar {
    playing: bool,
}

impl StatusBar {
    pub fn new(playing: bool) -> Self {
        Self { playing }
    }
}

const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

impl Widget for StatusBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let t = now_secs();
        for x in 0..area.width {
            let (ch, color) = if self.playing {
                bar_at(t, x as usize)
            } else {
                (LEVELS[1], Color::DarkGray)
            };
            if let Some(cell) = buf.cell_mut((area.x + x, area.y)) {
                cell.set_char(ch).set_fg(color);
            }
        }
    }
}

fn bar_at(t: f64, x: usize) -> (char, Color) {
    let wave = (t * 3.0 + x as f64 * 0.35).sin() * 0.6
        + (t * 5.0 + x as f64 * 0.18).sin() * 0.4;
    let idx = (((wave + 1.0) / 2.0) * 7.0).round().clamp(0.0, 7.0) as usize;
    (LEVELS[idx], bar_color(idx))
}

/// 青色系亮度渐变：柱越高越亮，保持与整体冷色调一致。
fn bar_color(level: usize) -> Color {
    let v = 0.35 + (level as f64 / 7.0) * 0.65;
    hsv_to_rgb(190.0, 0.75, v)
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> Color {
    let c = v * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as i64 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    Color::Rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
use std::time::Duration;

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Gauge, Paragraph, Widget},
};

use crate::app::{MusicLibrary, RepeatMode};
use crate::player::MyPlayer;
/// 底部控制栏：进度条 + 播放/暂停 + 上一首/下一首 + 循环模式指示
pub struct ControlBar<'a> {
    player: &'a MyPlayer,
    repeat: &'a RepeatMode,
    lib:&'a MusicLibrary,
}

impl<'a> ControlBar<'a> {
    pub fn new(player: &'a MyPlayer, repeat: &'a RepeatMode,lib:&'a MusicLibrary) -> Self {
        Self {
            player,
            repeat,
            lib,
        }
    }
}

impl Widget for ControlBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title(" 控制栏 ");
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 8 || inner.height < 3 {
            return;
        }

        let rows = Layout::vertical([
            Constraint::Length(1), // 歌曲信息 / 时长
            Constraint::Length(1), // 进度条
            Constraint::Min(1),    // 控制按钮
        ])
        .split(inner);

        self.render_info(rows[0], buf);
        self.render_progress(rows[1], buf);
        self.render_controls(rows[2], buf);
    }
}

impl ControlBar<'_> {
    fn render_info(&self, area: Rect, buf: &mut Buffer) {
        let time = format!(
            "{} / {}",
            fmt_duration(self.player.position()),
            fmt_duration(self.player.duration()),
        );
        let title = match self.player.get_playing_info(self.lib){
            Some((title,artist)) =>{
                let title = Span::styled(
                    title,
                    Style::default().add_modifier(Modifier::BOLD)
                );
                let artist = Span::styled(artist,
                     Style::default().fg(Color::Gray)
                    );
                Line::from(vec![
                    Span::styled("♪ ", Style::default().fg(Color::Magenta)),
                    title,
                    artist,
                ])
            }
            None => Line::from(vec![
                Span::styled("♪ ", Style::default().fg(Color::Magenta)),
                Span::styled("未在播放", Style::default().fg(Color::Gray)),
            ]),
        };

        let right = time;

        let split = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(right.len() as u16),
        ])
        .split(area);
        Paragraph::new(title).render(split[0], buf);
        Paragraph::new(right)
            .alignment(Alignment::Right)
            .style(Style::default().fg(Color::Gray))
            .render(split[1], buf);
    }

    fn render_progress(&self, area: Rect, buf: &mut Buffer) {
        let ratio = self.player.progress().clamp(0.0, 1.0);
        Gauge::default()
            .gauge_style(Style::default().fg(Color::Cyan))
            .ratio(ratio)
            .label(format!("{:>3.0}%", ratio * 100.0))
            .render(area, buf);
    }

    fn render_controls(&self, area: Rect, buf: &mut Buffer) {
        let prev = Span::styled(" [<<] ", Style::default().fg(Color::Gray));

        // 播放/暂停图标随播放状态切换
        let (play_icon, fg) = if self.player.is_paused() {
            ("[ Paused ]", Color::Yellow)
        } else if self.player.is_playing() {
            ("[ Playing ]", Color::Green)
        } else {
            ("[ Idle ]", Color::DarkGray)
        };
        let play = Span::styled(
            play_icon.to_string(),
            Style::default().fg(fg).add_modifier(Modifier::BOLD),
        );

        let next = Span::styled(" [>>] ", Style::default().fg(Color::Gray));
        let divider = Span::styled("  |  ", Style::default().fg(Color::DarkGray));

        // 循环模式指示（跟随 RepeatMode 当前值）
        let repeat_label = match self.repeat {
            RepeatMode::Sequential => "顺序",
            RepeatMode::Random => "随机",
            RepeatMode::All => "全部循环",
            RepeatMode::One => "单曲循环",
        };
        let repeat = Span::styled(
            format!("[↻ {repeat_label}]"),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        );
        let quality = format_quality(self.player.get_playing_quality(self.lib));
        let volume = Span::styled(format!("                {}         音量: {:.0}%",quality,self.player.volume()*100.0), Style::default());
        let line = Line::from(vec![prev, play, next, divider, Span::raw("模式 "), repeat,volume]);
        Paragraph::new(line).render(area, buf);
    }
}

/// 把时长格式化为 `mm:ss`
fn fmt_duration(d: Duration) -> String {
    let total = d.as_secs();
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// 把采样率/位深组合成 `44.1kHz / 16bit` 形式的字符串（缺失部分自动省略）。
fn format_quality(q: Option<(Option<u32>, Option<u32>)>) -> String {
    match q {
        Some((sr, bd)) => {
            let mut parts: Vec<String> = Vec::new();
            if let Some(v) = sr {
                parts.push(format_sample_rate(v));
            }
            if let Some(v) = bd {
                parts.push(format!("{v}bit"));
            }
            parts.join(" / ")
        }
        None => String::new(),
    }
}

/// 采样率转 `kHz`：整千显示整数（`48kHz`），否则保留一位小数（`44.1kHz`）。
fn format_sample_rate(sr: u32) -> String {
    if sr % 1000 == 0 {
        format!("{}kHz", sr / 1000)
    } else {
        format!("{:.1}kHz", sr as f64 / 1000.0)
    }
}
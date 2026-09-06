use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Paragraph, Widget},
};
use unicode_width::UnicodeWidthChar;

/// 一行歌词：`time` 为起始时间（秒），`text` 为歌词文本。
#[derive(Debug, Clone)]
pub struct LyricLine {
    pub time: f64,
    pub text: String,
}

/// 解析单个时间标签，`mm:ss`、`mm:ss.xx`、`mm:ss:xx` 均可。
fn parse_lrc_time(tag: &str) -> Option<f64> {
    let mut parts = tag.split(':');
    let min: f64 = parts.next()?.trim().parse().ok()?;
    let sec: f64 = parts.next()?.trim().parse().ok()?;
    if let Some(frac) = parts.next() {
        let frac: f64 = frac.trim().parse().ok()?;
        Some(min * 60.0 + sec + frac / 100.0)
    } else {
        Some(min * 60.0 + sec)
    }
}

/// 解析整份 LRC 内容为按时间排序的歌词行。
pub fn parse_lrc(content: &str) -> Vec<LyricLine> {
    let mut out: Vec<LyricLine> = Vec::new();
    let mut offset_ms: f64 = 0.0;

    for raw in content.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        let mut rest = line;
        let mut times: Vec<f64> = Vec::new();
        let mut is_meta = false;

        // 持续剥离行首的 `[tag]`
        loop {
            let Some(after_open) = rest.strip_prefix('[') else {
                break;
            };
            let Some(close) = after_open.find(']') else {
                break;
            };
            let tag = &after_open[..close];
            let after = &after_open[close + 1..];

            if let Some(off) = tag.strip_prefix("offset:") {
                if let Ok(ms) = off.trim().parse::<f64>() {
                    offset_ms = ms;
                }
                rest = after;
                continue;
            }
            if let Some(t) = parse_lrc_time(tag) {
                times.push(t);
                rest = after;
                continue;
            }
            // 非时间、非 offset 的标签（如 ti/ar/al/by），本行是元数据行，跳过。
            is_meta = true;
            break;
        }

        if is_meta || times.is_empty() {
            continue;
        }

        let text = rest.trim().to_string();
        for t in times {
            out.push(LyricLine {
                time: t + offset_ms / 1000.0,
                text: text.clone(),
            });
        }
    }

    out.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// 读取歌词文件内容，优先 UTF-8，失败时回退 GBK（中文歌词常见编码）。
fn read_lyrics_file(path: &str) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    if bytes.is_empty() {
        return String::new();
    }
    match String::from_utf8(bytes.clone()) {
        Ok(s) => s,
        Err(_) => {
            let (s, _, _) = encoding_rs::GBK.decode(&bytes);
            s.into_owned()
        }
    }
}

/// 读取并解析歌词文件；单曲场景只缓存最近一次解析结果。
fn load_lyrics(path: &str) -> Arc<Vec<LyricLine>> {
    static CACHE: OnceLock<Mutex<Option<(String, Arc<Vec<LyricLine>>)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().unwrap();

    if let Some((p, lines)) = guard.as_ref() {
        if p == path {
            return lines.clone();
        }
    }

    let content = read_lyrics_file(path);
    let lines = Arc::new(parse_lrc(&content));
    *guard = Some((path.to_string(), lines.clone()));
    lines
}

/// 定位到当前进度对应的歌词行索引（最后一个 `time <= pos` 的行）。
fn current_index(lines: &[LyricLine], pos: f64) -> usize {
    let mut idx = 0usize;
    for (i, l) in lines.iter().enumerate() {
        if l.time <= pos {
            idx = i;
        } else {
            break;
        }
    }
    idx
}

/// 按显示宽度把字符串自动换行成多行（正确处理 CJK 等宽字符）。
fn wrap_text(s: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![String::new()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_width = 0usize;
    for ch in s.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if cur_width + w > max_width && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
            cur_width = 0;
        }
        cur.push(ch);
        cur_width += w;
    }
    lines.push(cur);
    lines
}

/// 距当前行的样式：当前行加粗高亮，其余随距离渐隐。
fn line_style(dist: usize, is_current: bool) -> Style {
    if is_current {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        let fg = if dist == 1 { Color::Gray } else { Color::DarkGray };
        Style::default().fg(fg)
    }
}

/// 歌词滚动渲染 widget。持有当前歌词路径与播放进度。
pub struct LyricWidget {
    path: Option<Arc<str>>,
    position: Duration,
}

impl LyricWidget {
    pub fn new(path: Option<Arc<str>>, position: Duration) -> Self {
        Self { path, position }
    }
}

impl Widget for LyricWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title(" 歌词 ");
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 6 || inner.height < 3 {
            return;
        }

        let Some(path) = self.path.as_deref() else {
            render_hint(inner, buf, "无歌词");
            return;
        };

        let lines = load_lyrics(path);
        if lines.is_empty() {
            render_hint(inner, buf, "无歌词");
            return;
        }

        let pos = self.position.as_secs_f64();
        let current = current_index(&lines, pos);

        let visible = inner.height as usize;
        let width = inner.width as usize;

        // 把每条逻辑歌词行（加前缀后）按显示宽度展开成若干显示行，
        // 并记录每个显示行所属的逻辑行索引，用于后续样式与居中。
        let mut row_li: Vec<usize> = Vec::with_capacity(lines.len());
        let mut row_text: Vec<String> = Vec::new();
        for li in 0..lines.len() {
            let prefix = if li == current { "❯ " } else { "  " };
            let wrapped = wrap_text(&format!("{prefix}{}", lines[li].text), width);
            for t in wrapped {
                row_li.push(li);
                row_text.push(t);
            }
        }

        // 当前逻辑行的第一条显示行尽量居中；顶部/底部越界时贴边。
        let total = row_text.len();
        let current_first = row_li.iter().position(|&li| li == current).unwrap_or(0);
        let half = visible / 2;
        let mut start = current_first.saturating_sub(half);
        if start + visible > total {
            start = total.saturating_sub(visible);
        }
        let end = (start + visible).min(total);

        let mut rendered: Vec<Line> = Vec::with_capacity(visible);
        for r in start..end {
            let li = row_li[r];
            let dist = (li as isize - current as isize).unsigned_abs();
            let style = line_style(dist, li == current);
            rendered.push(Line::styled(row_text[r].clone(), style));
        }

        Paragraph::new(rendered)
            .alignment(Alignment::Center)
            .render(inner, buf);
    }
}

fn render_hint(area: Rect, buf: &mut Buffer, msg: &str) {
    Paragraph::new(msg)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray))
        .render(area, buf);
}
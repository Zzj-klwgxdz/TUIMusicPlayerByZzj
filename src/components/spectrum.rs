//! - [`TapSource`]：透传 Source，运行在 rodio 音频线程，顺手把样本降混成单声道拷进共享缓冲；
//! - [`SpectrumBuffer`]：仅保留最近若干秒的**单声道**滑窗样本，内存恒定；
//! - [`SpectrumAnalyzer`]：UI 线程读缓冲 → Hann 窗 → f64 FFT → 输出对数频率/对数幅值的频谱点；
//! - [`SpectrumChart`]：用 `ratatui::Chart` 渲染成对数频率轴的频谱曲线。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    symbols::Marker,
    widgets::{Axis, Block, Chart, Dataset, GraphType, Widget},
};
use rodio::source::SeekError;
use rodio::{ChannelCount, SampleRate, Source};
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

// 共享滑窗缓冲

/// 音频线程写入、UI 线程读取的单声道滑动缓冲。
/// 内存上限固定 = `capacity` 个 f32 采样，超出丢弃最旧数据。
pub struct SpectrumBuffer {
    inner: Mutex<VecDeque<f32>>,
    capacity: usize,
}

impl SpectrumBuffer {
    /// `capacity_seconds` 保留秒数；`sample_rate` 单声道采样率（默认 44100）。
    pub fn new(capacity_seconds: f32, sample_rate: u32) -> Self {
        let capacity = (capacity_seconds * sample_rate as f32) as usize;
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: capacity.max(1024),
        }
    }

    /// 音频线程写入一个降混后的单声道样本。
    /// 用 `try_lock`，拿不到锁就丢弃此样本，避免阻塞音频线程。
    fn push(&self, s: f32) {
        if let Ok(mut inner) = self.inner.try_lock() {
            if inner.len() >= self.capacity {
                inner.pop_front();
            }
            inner.push_back(s);
        }
    }

    /// 清空并复位（切歌 / seek 时由 UI 侧调用）。
    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.clear();
        }
    }

    /// 取最近 `n` 个样本（不删除，滑动窗口内的样本会被容量淘汰）。
    /// 数据不足时返回已取到的部分，由调用方决定是否用 0 补齐。
    pub fn take(&self, n: usize) -> Vec<f32> {
        let inner = self.inner.lock().unwrap();
        let len = inner.len();
        let wanted = n.min(len);
        let start = len - wanted;
        inner.range(start..).copied().collect()
    }
}

/// 共享句柄
pub type SharedSpectrum = Arc<SpectrumBuffer>;

// 透传取样 Source

/// 包装任意 `Source`，逐样本透传给 rodio 的同时，降混为单声道拷进 [`SpectrumBuffer`]。
/// 自身是纯标量透传，不碰 I/O 不解码，近似零开销。
pub struct TapSource<S>
where
    S: Source<Item = f32> + Send + 'static,
{
    inner: S,
    tap: SharedSpectrum,
    channels: usize,
    acc: f32,
    frame: usize,
}

impl<S> TapSource<S>
where
    S: Source<Item = f32> + Send + 'static,
{
    pub fn new(inner: S, tap: SharedSpectrum) -> Self {
        let channels = inner.channels().get() as usize;
        Self {
            inner,
            tap,
            channels: channels.max(1),
            acc: 0.0,
            frame: 0,
        }
    }
}

impl<S> Iterator for TapSource<S>
where
    S: Source<Item = f32> + Send + 'static,
{
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let s = self.inner.next()?;
        // 每声道样本到达时累加,凑满一个音频帧再降混写入
        self.acc += s;
        self.frame += 1;
        if self.frame >= self.channels {
            let mono = self.acc / self.channels as f32;
            self.acc = 0.0;
            self.frame = 0;
            self.tap.push(mono);
        }
        Some(s)
    }
}

impl<S> Source for TapSource<S>
where
    S: Source<Item = f32> + Send + 'static,
{
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(pos)?;
        // seek 后底层从新帧边界开始，重置降混累积状态，避免首帧频谱错位
        self.acc = 0.0;
        self.frame = 0;
        Ok(())
    }
}

// FFT 频谱分析

/// 频谱分析器：从 [`SpectrumBuffer`] 取窗，加 Hann 窗 + 归一化后做 f64 FFT，
/// 输出 `(ln(freq), ln(magnitude))` 的频谱点，频率单位 Hz。
pub struct SpectrumAnalyzer {
    tap: SharedSpectrum,
    fft_size: usize,
    half: usize,
    sample_rate: u32,
    window_enabled: bool,
    plan: Arc<dyn rustfft::Fft<f64>>,
    window: Vec<f64>,
    gain: f64,
    bands: usize,
    f_low: f64,
    f_high: f64,
}

impl SpectrumAnalyzer {
    pub fn new(tap: SharedSpectrum, fft_size: usize, sample_rate: u32) -> Self {
        let mut planner = FftPlanner::new();
        let plan = planner.plan_fft_forward(fft_size);
        // Hann 窗
        let mut window = vec![0.0; fft_size];
        for i in 0..fft_size {
            window[i] = 0.5
                * (1.0
                    - (2.0 * std::f64::consts::PI * i as f64 / fft_size as f64).cos());
        }
        Self {
            tap,
            fft_size,
            half: fft_size / 2,
            sample_rate,
            window_enabled: true,
            plan,
            window,
            gain: 1.0,
            bands: 64,
            f_low: 20.0,
            f_high: 20000.0,
        }
    }

    /// 调节整体增益（缩放入 FFT 前的样本，进而抬升 ln 幅值曲线）。
    pub fn gain(mut self, g: f64) -> Self {
        self.gain = g;
        self
    }

    /// 是否启用 Hann 窗（默认启用）。
    pub fn window(mut self, enabled: bool) -> Self {
        self.window_enabled = enabled;
        self
    }

    /// 设置对数频带数量（= 对数轴上数据点数量，越多越密）。
    pub fn bands(mut self, n: usize) -> Self {
        self.bands = n.max(1);
        self
    }

    /// 设置对数分桶的频率范围（Hz）。
    pub fn freq_range(mut self, low: f64, high: f64) -> Self {
        self.f_low = low.max(1.0);
        self.f_high = high;
        self
    }

    /// 频谱的实际采样率
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 清空底层缓冲（切歌 / seek 时调用）
    pub fn clear(&self) {
        self.tap.clear();
    }

    /// FFT 窗口大小（供渲染侧估算 y 上限）
    pub fn fft_size(&self) -> usize {
        self.fft_size
    }

    /// 计算当前频谱，输出 `(x = ln(freq), y = ln(magnitude))` 数据点。
    /// 静音/无数据时返回空。
    pub fn spectrum(&self) -> Vec<(f64, f64)> {
        let samples = self.tap.take(self.fft_size);
        if samples.is_empty() {
            return Vec::new();
        }

        // 转 f64，不足补 0
        let mut chunk: Vec<f64> = (0..self.fft_size)
            .map(|i| samples.get(i).copied().unwrap_or(0.0) as f64)
            .collect();

        // Hann 窗
        if self.window_enabled {
            for (i, v) in chunk.iter_mut().enumerate() {
                *v *= self.window[i];
            }
        }

        // 归一化到 ±1（静音时峰值记作 1，避免除零）
        let mut peak = 0.0f64;
        for &v in &chunk {
            let a = v.abs();
            if a > peak {
                peak = a;
            }
        }
        if peak < 1.0 {
            peak = 1.0;
        }

        let mut buf: Vec<Complex<f64>> = chunk
            .iter()
            .map(|&x| Complex::new(x / peak * self.gain, 0.0))
            .collect();
        self.plan.process(&mut buf);

        let resolution = self.sample_rate as f64 / self.fft_size as f64;

        // 对数分桶：把线性 FFT bin 聚合到对数间隔的频带，使 x 轴点在对数轴上均匀分布
        let nyquist = self.sample_rate as f64 / 2.0;
        let f_low = self.f_low.max(resolution);
        let f_high = self.f_high.min(nyquist);
        if f_high <= f_low {
            return Vec::new();
        }

        let log_ratio = (f_high / f_low).ln();
        let bands = self.bands;
        let mut band_max = vec![0.0f64; bands];
        let mut band_count = vec![0usize; bands];

        for (i, c) in buf.iter().enumerate().take(self.half).skip(1) {
            let freq = i as f64 * resolution;
            if freq < f_low {
                continue;
            }
            if freq >= f_high {
                break; // freq 随索引单调递增，越界即可停止
            }
            let k = ((freq / f_low).ln() / log_ratio * bands as f64).floor() as usize;
            let k = k.min(bands - 1);
            let mag = c.norm();
            if mag > band_max[k] {
                band_max[k] = mag;
            }
            band_count[k] += 1;
        }

        // 低频区 bin 稀疏，会出现没有 bin 落入的空频带（图上跌成谷底/孤立尖峰），
        // 用左右最近的非空频带线性插值填平，使低频段曲线连续。
        let mags = interpolate_bands(&band_max, &band_count);

        // 每个频带输出一个点：x = ln(中心频率)，y = ln(带内峰值幅值)
        let mut out = Vec::with_capacity(bands);
        for (k, &m) in mags.iter().enumerate() {
            let center = f_low * (f_high / f_low).powf((k as f64 + 0.5) / bands as f64);
            out.push((center.ln(), m.max(1e-6).ln()));
        }
        out
    }
}

/// 对没有 bin 落入的空频带（低频区 bin 稀疏导致），用左右最近的非空频带做线性插值填平。
fn interpolate_bands(peaks: &[f64], counts: &[usize]) -> Vec<f64> {
    let n = peaks.len();
    let mut out = peaks.to_vec();
    let mut i = 0;
    while i < n {
        if counts[i] > 0 {
            i += 1;
            continue;
        }
        // 连续空频带区间 [start, end)
        let start = i;
        while i < n && counts[i] == 0 {
            i += 1;
        }
        let end = i;
        let left = (0..start).rev().find(|&j| counts[j] > 0);
        let right = (end..n).find(|&j| counts[j] > 0);
        match (left, right) {
            (Some(l), Some(r)) => {
                let (lv, rv) = (out[l], out[r]);
                for j in start..end {
                    let t = (j - l) as f64 / (r - l) as f64;
                    out[j] = lv + (rv - lv) * t;
                }
            }
            (Some(l), None) => {
                let lv = out[l];
                for j in start..end {
                    out[j] = lv;
                }
            }
            (None, Some(r)) => {
                let rv = out[r];
                for j in start..end {
                    out[j] = rv;
                }
            }
            (None, None) => {}
        }
    }
    out
}

// 渲染 widget

/// 频谱曲线渲染：`Chart` + 对数频率轴，不绘制参考刻度线（保持画面干净）。
pub struct SpectrumChart<'a> {
    analyzer: &'a SpectrumAnalyzer,
    fg: Color,
    title: &'a str,
}

impl<'a> SpectrumChart<'a> {
    pub fn new(analyzer: &'a SpectrumAnalyzer) -> Self {
        Self {
            analyzer,
            fg: Color::Cyan,
            title: "Spectrum",
        }
    }
    pub fn color(mut self, c: Color) -> Self {
        self.fg = c;
        self
    }
    pub fn title(mut self, t: &'a str) -> Self {
        self.title = t;
        self
    }
}

impl Widget for &SpectrumChart<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title(self.title);
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width < 8 || inner.height < 3 {
            return;
        }
        let spectrum = self.analyzer.spectrum();

        let x_low = 20.0f64.ln();
        let x_high = 20000.0f64.ln();
        let y_max = (self.analyzer.fft_size() as f64).ln().max(1.0);

        let dataset = Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(self.fg))
            .data(&spectrum);

        let chart = Chart::new(vec![dataset])
            .x_axis(Axis::default().bounds([x_low, x_high]))
            .y_axis(Axis::default().bounds([0.0, y_max]));
        chart.render(inner, buf);
    }
}
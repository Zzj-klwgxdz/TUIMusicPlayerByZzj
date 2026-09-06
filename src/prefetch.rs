use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use rodio::{ChannelCount, SampleRate, Source};
use rodio::source::SeekError;

/// 固定时长滑动预取缓冲。
///
/// 后台线程持续从内层 source 读取/解码，往共享缓冲充到 `high` 水位；
/// 音频线程只从缓冲弹出，绝不接触磁盘/解码。水位低于 `low` 时后台补满。
/// 缓冲只保留约 `buffer_seconds`
/// 读盘+解码被隔离到后台线程，只要补片耗时 < 缓冲剩余时长就不易断流。
pub struct PrefetchSource<S>
where
    S: Source + Send + 'static,
    S::Item: Send,
{
    shared: Arc<Shared<S>>,
    channels: ChannelCount,
    sample_rate: SampleRate,
    total_duration: Option<Duration>,
    handle: Option<thread::JoinHandle<()>>,
}

struct Shared<S: Source> {
    inner: Mutex<S>,
    buffer: Mutex<VecDeque<S::Item>>,
    version: AtomicU64,
    eof: AtomicBool,
    running: AtomicBool,
    low: usize,
    high: usize,
}

impl<S> PrefetchSource<S>
where
    S: Source + Send + 'static,
    S::Item: Send,
{
    /// `buffer_seconds`：预留的预取时长（秒），越大抗卡顿up ，RAM占用up
    pub fn new(inner: S, buffer_seconds: u64) -> Self {
        let channels = inner.channels();
        let sample_rate = inner.sample_rate();
        let total_duration = inner.total_duration();

        let samples_per_second = u64::from(channels.get()) * u64::from(sample_rate.get());
        let high = (samples_per_second * buffer_seconds) as usize;
        let low = (high / 2).max(1); // 低于一半触发后台补片

        let shared = Arc::new(Shared {
            inner: Mutex::new(inner),
            buffer: Mutex::new(VecDeque::new()),
            version: AtomicU64::new(0),
            eof: AtomicBool::new(false),
            running: AtomicBool::new(true),
            low,
            high,
        });

        let worker = Arc::clone(&shared);
        let handle = thread::spawn(move || {
            let mut chunk: Vec<S::Item> = Vec::new();
            while worker.running.load(Ordering::Relaxed) {
                let need = worker.buffer.lock().unwrap().len() < worker.low;
                if need {
                    chunk.clear();
                    let v0 = worker.version.load(Ordering::SeqCst);
                    {
                        // 只在内层 source 上做 I/O+解码，不占用 buffer 锁
                        let mut inner = worker.inner.lock().unwrap();
                        while chunk.len() < worker.high {
                            match inner.next() {
                                Some(s) => chunk.push(s),
                                None => {
                                    worker.eof.store(true, Ordering::SeqCst);
                                    break;
                                }
                            }
                        }
                    }
                    // 期间若发生过 seek（gen 变化），丢弃这批旧数据
                    if worker.version.load(Ordering::SeqCst) == v0 {
                        let mut buf = worker.buffer.lock().unwrap();
                        buf.extend(chunk.drain(..));
                    }
                }
                thread::sleep(Duration::from_millis(5));
            }
        });

        Self {
            shared,
            channels,
            sample_rate,
            total_duration,
            handle: Some(handle),
        }
    }
}

impl<S> Iterator for PrefetchSource<S>
where
    S: Source + Send + 'static,
    S::Item: Send,
{
    type Item = S::Item;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(s) = self.shared.buffer.lock().unwrap().pop_front() {
                return Some(s);
            }
            if self.shared.eof.load(Ordering::SeqCst) {
                return None;
            }
            thread::yield_now(); // 缓冲区暂时见底但后台还在补，让出
        }
    }
}

impl<S> Source for PrefetchSource<S>
where
    S: Source + Send + 'static,
    S::Item: Send,
{
    fn current_span_len(&self) -> Option<usize> {
        Some(self.shared.buffer.lock().unwrap().len())
    }

    fn channels(&self) -> ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        // 先让底层真正 seek，成功后再清缓冲；失败时不破坏已缓冲数据
        self.shared.inner.lock().unwrap().try_seek(pos)?;
        self.shared.version.fetch_add(1, Ordering::SeqCst);
        self.shared.buffer.lock().unwrap().clear();
        self.shared.eof.store(false, Ordering::SeqCst);
        Ok(())
    }
}

impl<S> Drop for PrefetchSource<S>
where
    S: Source + Send + 'static,
    S::Item: Send,
{
    fn drop(&mut self) {
        self.shared.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
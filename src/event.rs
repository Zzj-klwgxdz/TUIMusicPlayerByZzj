use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;
use crossterm::event::{Event as CrosstermEvent, KeyEvent, MouseEvent};
use futures::StreamExt;
/// Terminal events.
#[derive(Clone, Copy, Debug)]
pub enum Event {
    /// Terminal tick.
    Tick,
    /// Key press.
    Key(KeyEvent),
    /// Mouse click/scroll.
    Mouse(MouseEvent),
    // /// Terminal resize.
    // Resize(u16, u16),
}
/// 事件处理器，在后台任务中运行
pub struct EventLoop {
    /// 发送端，用于将事件发送给主循环
    // sender: UnboundedSender<Event>,
    /// 接收端，主循环从此读取事件
    pub receiver: UnboundedReceiver<Event>,
    /// 任务句柄，用于等待任务结束
    task: tokio::task::JoinHandle<()>,
    /// 取消令牌，用于停止事件循环
    cancellation_token: CancellationToken,
}

impl EventLoop {
    /// 创建新的事件循环实例，立即启动后台任务
    pub fn new(tick_rate: f64) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        let cancellation_token = CancellationToken::new();
        let task = tokio::spawn(Self::run_loop(
            sender.clone(),
            cancellation_token.clone(),
            tick_rate,
        ));

        Self {
            // sender,
            receiver,
            task,
            cancellation_token,
        }
    }

    /// 后台事件循环（运行在 tokio 任务中）
    async fn run_loop(sender: UnboundedSender<Event>,cancellation_token: CancellationToken,tick_rate: f64,){
        // 1. 创建异步事件流（来自 crossterm）
        let mut event_stream = crossterm::event::EventStream::new();
        // 2. 创建 tick 定时器
        let tick_duration = std::time::Duration::from_secs_f64(1.0 / tick_rate);
        let mut tick_interval = tokio::time::interval(tick_duration);

        loop {
            tokio::select! {
                // 取消信号
                _ = cancellation_token.cancelled() => {
                    break;
                }
                // Tick 定时器
                _ = tick_interval.tick() => {
                    if sender.send(Event::Tick).is_err() {
                        break; // 接收端已关闭，结束循环
                    }
                }
                // 键盘事件
                crossterm_event = event_stream.next() => {
                    match crossterm_event {
                        Some(Ok(CrosstermEvent::Key(key))) => {
                            if sender.send(Event::Key(key)).is_err() {
                                break;
                            }
                        }

                        Some(Ok(CrosstermEvent::Mouse(mouse_event))) => {
                            if sender.send(Event::Mouse(mouse_event)).is_err(){
                                break;
                            }
                        }
                        // 可以在这里处理其他事件类型，如 Resize
                        _ => {}
                    }
                }
            }
        }
    }

    /// 停止事件循环，并等待任务完成
    pub async fn stop(self) {
        self.cancellation_token.cancel();
        let _ = self.task.await;
    }
}

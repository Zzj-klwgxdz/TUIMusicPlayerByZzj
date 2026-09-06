use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture}, execute, terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::io;
use crate::event::{Event, EventLoop};
use color_eyre::Result;
pub type CrosstermTerminal = Terminal<CrosstermBackend<io::Stdout>>;

/// TUI 全局状态
pub struct Tui {
    /// 终端后端
    pub terminal: CrosstermTerminal,
    /// 事件循环
    pub event_loop: EventLoop,
}

impl Tui {
    /// 初始化 TUI：进入原始模式、备用屏幕，并启动事件循环
    pub fn new(tick_rate: f64) -> Result<Self> {
        // 设置终端
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture,EnableBracketedPaste)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        // 启动事件循环
        let event_loop = EventLoop::new(tick_rate);

        Ok(Self {
            terminal,
            event_loop,
        })
    }

    /// 获取事件接收器，主循环通过 `recv().await` 获取事件
    pub fn event_receiver(&mut self) -> &mut tokio::sync::mpsc::UnboundedReceiver<Event> {
        &mut self.event_loop.receiver
    }

    /// 绘制 UI
    pub fn draw<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(&mut ratatui::Frame),
    {
        let _result = self.terminal.draw(f)?;
        Ok(())
    }

    /// 恢复终端（退出备用屏幕、恢复原始模式）
    pub async fn restore(mut self) -> io::Result<()> {
        // 先停止事件循环（确保不再产生事件）
        self.event_loop.stop().await;
        // 恢复终端设置
        disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), DisableMouseCapture, LeaveAlternateScreen,DisableBracketedPaste)?;
        Ok(())
    }
}

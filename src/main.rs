use std::{path::Path, time::{Duration, SystemTime}};
use color_eyre::Result;
mod event;
mod tui;
mod app;
mod components;
mod ui;
mod player;
mod prefetch;
mod database;
use app::App;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseEvent};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt,util::SubscriberInitExt};
use crate::tui::Tui;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    //日志系统
    let file_appender = tracing_appender::rolling::daily("./logs", "app.log");
    let (non_blocking,_guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::Layer::new()
            .with_writer(non_blocking)
            .with_ansi(false)
        )
        .init();
    color_eyre::install()?;
    //清理3天前的日志
    clean_old_logs("./logs", 3).unwrap_or_else(|e|{tracing::info!("清理3天前的日志出现错误{:?}",e)});
    let mut tui = Tui::new(240.0)?; //帧率
    tracing::info!("程序初始化完成");
    let mut app = App::new().await?;
    loop {
        // 异步等待下一个事件
        if let Some(event) = tui.event_receiver().recv().await {
            match event {
                event::Event::Tick => {
                    app.check_audio_device().await;
                    if app.player.track_ended(app.was_playing){
                        app.handle_track_end().await;
                    }
                }
                event::Event::Key(KeyEvent{code, kind, ..}) => {
                    if code == KeyCode::Char('q') || code == KeyCode::Esc {
                        break;
                    }
                    if kind == KeyEventKind::Press{
                        app.handle_key(code).await?;
                    } 
                }
                event::Event::Mouse(MouseEvent{kind,..}) =>{
                    app.handle_mouse_event_kind(kind).await?;
                }
            }
            tui.draw(|frame| {
                let area = frame.area();
                app.render(frame, area);
            })?;
        }
    }
    tui.restore().await?;
    tracing::info!("============程序正常退出============");
    color_eyre::Result::Ok(())
}
fn clean_old_logs<P: AsRef<Path>>(path: P, retention_days: u64) -> std::io::Result<()> {
    let cutoff = SystemTime::now() - Duration::from_secs(retention_days * 24 * 60 * 60);
    let entries = std::fs::read_dir(path)?;
    for entry in entries {
        let entry = entry?;
        let metadata = entry.metadata()?;

        if metadata.is_file() {
            if let Ok(modified) = metadata.modified() {
                if modified < cutoff {
                    // 文件早于截止时间，执行删除
                    std::fs::remove_file(entry.path())?;
                    println!("Deleted: {:?}", entry.path());
                }
            }
        }
    }
    tracing::info!("已清理3天前的日志");
    Ok(())
}

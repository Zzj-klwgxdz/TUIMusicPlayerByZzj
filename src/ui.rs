use crossterm::event::KeyCode;
use crossterm::event::MouseEventKind;
use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::HorizontalAlignment::Left;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use ratatui::widgets::Cell;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Padding;
use ratatui::widgets::Row;
use ratatui::widgets::Scrollbar;
use ratatui::widgets::ScrollbarOrientation;
use ratatui::widgets::ScrollbarState;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;
use crate::App;
use crate::app::AppMode;
use crate::components::spectrum::{SpectrumAnalyzer, SpectrumChart};
use crate::database::modify_in_list_status_database;
use crate::components::control_bar_ui::ControlBar;
use crate::components::lyric::LyricWidget;
use crate::components::status_bar::StatusBar;
pub struct AppUIState {
    pub playlist_state: ListState,
    pub library_state: TableState,
    pub playlist_scrollbar_state: ScrollbarState,
    pub library_scrollbar_state: ScrollbarState,

}
impl AppUIState{
    pub fn new() -> Self{
        Self{
            playlist_state:ListState::default(),
            library_state:TableState::default(),
            playlist_scrollbar_state:ScrollbarState::default(),
            library_scrollbar_state:ScrollbarState::default()
        }
    }
}
impl App{
    pub fn render(&mut self,frame:&mut Frame,area:Rect){
        let block = Block::default()
            .style(Style::default())
            .title(Line::from("Music Player").alignment(Left))
            .border_type(BorderType::Rounded)
            .borders(Borders::ALL)
            .padding(Padding::new(1,1,1,0));
        let inner_area = block.inner(area);
        frame.render_widget(block, area);
        match self.current_screen{
            AppMode::Main =>{
                self.render_main(frame, inner_area);
            },
            AppMode::Library =>{
                self.render_library(frame, inner_area);
            }
        }
            
    }
    pub fn render_main(&mut self,frame:&mut Frame,area:Rect){
        //最外侧布局
        let out_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![
                Constraint::Length(1),//上方状态栏
                Constraint::Fill(1),//中间
                Constraint::Length(1)//下方提示
            ])
            .split(area);
        let middle_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![
                Constraint::Percentage(15),//playlist
                Constraint::Length(1),//scrollbar
                Constraint::Fill(1),//main
                Constraint::Percentage(20)//lyric_area
            ])
            .split(out_layout[1]);
        let main_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![
                Constraint::Fill(1),//频谱
                Constraint::Length(5)//control_bar
            ])
            .split(middle_layout[2]);
        //渲染list
        let listitem:Vec<_> = self.play_queue.lock().unwrap().queue.iter()
                .map(|index|{
                    let song = &self.music_list.tracks[*index];
                    ListItem::new(song.title.as_str())
                })
                .collect();
        let playlist = List::new(listitem).block(Block::bordered().title("播放列表"))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        self.app_ui_state.playlist_state.select(self.play_queue.lock().unwrap().current_index);
        frame.render_stateful_widget(playlist, middle_layout[0], &mut self.app_ui_state.playlist_state);
        //Scrollbar
        let total = self.play_queue.lock().unwrap().queue.len();
        let selected = self.app_ui_state.playlist_state.selected().unwrap_or(0);
        self.app_ui_state.playlist_scrollbar_state = self.app_ui_state.playlist_scrollbar_state
            .content_length(total)
            .position(selected);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight).thumb_style(Color::Gray);
        frame.render_stateful_widget(scrollbar, middle_layout[1], &mut self.app_ui_state.playlist_scrollbar_state);
        //渲染频谱
        let analyzer = SpectrumAnalyzer::new(
            self.player.tap(),
            16384,
            self.player.spectrum_sample_rate(),
        ).bands(128);
        frame.render_widget(&SpectrumChart::new(&analyzer), main_layout[0]);
        //渲染control_bar
        frame.render_widget(
            ControlBar::new(&self.player,  &self.repeatmode,&self.music_list),
            main_layout[1], 
        );
        //渲染歌词
        let lyric_path = self.player.get_current_lyric_path(&self.music_list);
        let position = self.player.position();
        frame.render_widget(
            LyricWidget::new(lyric_path, position),
            middle_layout[3],
        );
        //渲染操作提示
        let hint_line = Line::from(vec![
            Span::styled("Enter ", Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
            Span::raw("开始播放    "),
            Span::styled("S ", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)),
            Span::raw("停止播放    "),
            Span::styled("C ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            Span::raw("播放模式切换    "),
            Span::styled("Space ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("暂停/继续    "),
            Span::styled("↑ / ↓ ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw("选择音乐    "),
            Span::styled("← / → ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw("快进/快退    "),
            Span::styled("+ / - ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw("音量调节    "),
            Span::styled("L ", Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD)),
            Span::raw("音乐库页面    "),
            Span::styled("Esc / Q ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::raw("退出程序    "),
        ]);
        frame.render_widget(hint_line, out_layout[2]); 

        //上方状态栏动画（播放时跳动，暂停/停止时静止）
        frame.render_widget(StatusBar::new(self.player.is_playing()), out_layout[0]);
    }
    pub fn render_library(&mut self,frame:&mut Frame,area:Rect){
        let out_layout = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(1)
        ]).split(area);
        let layout = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(1)
        ]).split(out_layout[0]);
        let list_area = layout[0];
        let scrollbar_area = layout[1];
        //Table
        let header = Row::new(vec![" ","标题","音乐家","专辑", "时长", "路径"])
            .style(Style::default().add_modifier(Modifier::BOLD))
            .height(1);
        let rows = self.music_list.tracks.iter().map(|song| {
            Row::new(vec![
                Cell::from(Line::from(format!("{}",if song.in_list == 1 {'*'} else {' '})).bold()),
                Cell::from(song.title.as_str()),
                Cell::from(song.artist.clone().unwrap_or("Unknow".into())),
                Cell::from(song.album.clone().unwrap_or("Unknow".into())),
                Cell::from(format_duration(song.duration)),
                Cell::from(song.path.as_ref()),
            ])
        });
        let table = Table::new(
            rows,
            vec![
                Constraint::Length(2),// 是否在播放列表标记
                Constraint::Percentage(15),// 标题
                Constraint::Percentage(15),//音乐家
                Constraint::Percentage(15),//专辑
                Constraint::Percentage(5),// 时长 mm:ss
                Constraint::Fill(1),// 路径占满剩余
            ],
        )
            .header(header)
            .block(Block::bordered().title("全部音乐").title_alignment(Alignment::Center))
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .column_spacing(2);
        frame.render_stateful_widget(table, list_area, &mut self.app_ui_state.library_state);
        //scrollbar
        let total = self.music_list.tracks.len();
        let selected = self.app_ui_state.library_state.selected().unwrap_or(0);
        self.app_ui_state.library_scrollbar_state = self.app_ui_state.library_scrollbar_state
            .content_length(total)
            .position(selected);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight).thumb_style(Color::Gray);
        frame.render_stateful_widget(scrollbar, scrollbar_area, &mut self.app_ui_state.library_scrollbar_state);

        //渲染操作提示   
        let hint_line = Line::from(vec![
            Span::styled("I ", Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD)),
            Span::raw("导入音乐或歌词文件夹    "),
            Span::styled("Space ", Style::default().fg(Color::LightYellow).add_modifier(Modifier::BOLD)),
            Span::raw("添加/移除播放列表    "),
            Span::styled("↑ / ↓ ", Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
            Span::raw("选择音乐    "),
            Span::styled("M ", Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD)),
            Span::raw("主播放页面    "),
            Span::styled("Esc / Q ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::raw("退出程序    "),
        ]);
        frame.render_widget(hint_line, out_layout[1]);
    }
    pub async fn handle_key(&mut self,code:KeyCode) ->color_eyre::Result<()>{
        if self.current_screen == AppMode::Library{
            match code{
                KeyCode::Up => {
                    self.app_ui_state.library_state.select_previous();
                }
                KeyCode::Down =>{
                    self.app_ui_state.library_state.select_next();
                }
                //add music to list or remove it from list
                KeyCode::Char(' ') =>{
                    self.modify_in_list_status().await;
                }
                //main mode
                KeyCode::Char('m') =>{
                    // 仅当 current_index 还没初始化且队列非空时才初始化，避免回 Main 时误停正在播放的歌
                    let needs_init = {
                        let q = self.play_queue.lock().unwrap();
                        q.current_index.is_none() && !q.queue.is_empty()
                    };
                    if needs_init {
                        self.player.modify_current(0);
                        self.was_playing = false;
                    }
                    self.player.update_playing_index(&self.music_list);
                    self.current_screen = AppMode::Main;
                }
                KeyCode::Char('i') =>{
                    self.import_music().await?;
                }
                _ => {}
            }
        }else if self.current_screen == AppMode::Main{
            match code{
                KeyCode::Char('l') =>{
                    self.current_screen = AppMode::Library;
                }
                KeyCode::Up =>{
                    self.player.previous_music();
                }
                KeyCode::Down =>{
                    self.player.next_music();
                }
                KeyCode::Char(' ') =>{
                    if self.player.is_paused(){
                        self.player.resume();
                        self.was_playing = true;
                    }else{
                        self.player.pause();
                        self.was_playing = false;
                    }
                }
                KeyCode::Char('c') =>{
                    self.repeatmode = self.repeatmode.change()
                }
                KeyCode::Enter =>{
                    self.player.play(&self.music_list).await?;
                    self.was_playing = true;
                }
                KeyCode::Char('s') =>{
                    self.player.stop();
                    self.was_playing = false;
                }
                KeyCode::Right =>{
                    self.player.seek_forward(5);
                }
                KeyCode::Left =>{
                    self.player.seek_backward(5);
                }
                KeyCode::Char('+') =>{
                    self.player.volume_up();
                }
                KeyCode::Char('-') =>{
                    self.player.volume_down();
                }
                _ => {}
            }
        }
        Ok(())
    }
    pub async fn handle_mouse_event_kind(&mut self,kind:MouseEventKind) ->color_eyre::Result<()>{
        match kind{
            MouseEventKind::ScrollDown => {
                self.handle_key(KeyCode::Down).await?
            }
            MouseEventKind::ScrollUp => {
                self.handle_key(KeyCode::Up).await?
            }
            _ =>{}
        }
        Ok(())
    }
    async fn modify_in_list_status(&mut self){
        if let Some(idx) = self.app_ui_state.library_state.selected(){
            let (id, new) = match self.music_list.tracks.get_mut(idx){
                Some(song) => {
                    let new = if song.in_list == 1 { 0 } else { 1 };
                    song.in_list = new;
                    (song.id, new)
                }
                None => return
            };
            match id{
                Some(id) =>{
                    match modify_in_list_status_database(&self.database, id, new).await{
                        Ok(rep) => {
                            tracing::info!("{rep:?}");
                            self.play_queue.lock().unwrap().update(&self.music_list);
                        }
                        Err(e) => tracing::error!("{}",e.to_string())
                    }
                }
                None =>{tracing::error!("modify_in_list_status无法找到song.id")}
            }   
        }else{}
    }
    
}

/// 把秒数格式化为 `m:ss` 或 `mm:ss`
pub fn format_duration(duration_secs: i64) -> String {
    let minutes = duration_secs / 60;
    let seconds = duration_secs % 60;
    format!("{}:{:02}", minutes, seconds)
}
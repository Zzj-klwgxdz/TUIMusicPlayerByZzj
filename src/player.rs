use std::{borrow::Cow, fs::File,sync::{Arc,Mutex,atomic::{AtomicBool, Ordering}}, time::Duration};
use color_eyre::Result;
use rand::RngExt;
use rayon::iter::{IndexedParallelIterator, IntoParallelRefIterator, ParallelIterator};
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use crate::app::{App, MusicLibrary, PlayQueue, RepeatMode};
use crate::prefetch::PrefetchSource;
use crate::components::spectrum::{SharedSpectrum, SpectrumBuffer, TapSource};
pub struct MyPlayer{
    _sink:MixerDeviceSink,
    player:Player,
    duration:Duration,
    play_list:Arc<Mutex<PlayQueue>>,
    tap:SharedSpectrum,
    sample_rate:u32,
    playing_index:Option<usize>, //正在播放的音乐play_list索引,用于control_bar音乐信息的显示

    volume:f32,
    current_path:Option<(Arc<str>,Duration)>, //当前正在播放的音源路径与时长，用于音频设备热切换时重建
    device_lost:Arc<AtomicBool>, //由音频流错误回调置位，标记当前输出设备已丢失
}
impl MyPlayer{
    //打开默认输出设备，并挂载自定义错误回调：设备丢失时静默置位标志，避免向 stderr 打印破坏 TUI
    fn build_sink(device_lost: Arc<AtomicBool>) -> Result<MixerDeviceSink> {
        let callback = move |err: rodio::cpal::StreamError| match err {
            rodio::cpal::StreamError::DeviceNotAvailable => {
                device_lost.store(true, Ordering::SeqCst);
            }
            other => {
                tracing::warn!("音频流错误: {other}");
            }
        };
        let mut sink = DeviceSinkBuilder::from_default_device()?
            .with_error_callback(callback)
            .open_stream()?;
        sink.log_on_drop(false);
        Ok(sink)
    }

    pub fn try_new(playlist: Arc<Mutex<PlayQueue>>) -> Result<Self>{
        let device_lost = Arc::new(AtomicBool::new(false));
        let sink = Self::build_sink(device_lost.clone())?;
        let player = Player::connect_new(sink.mixer());
        Ok(Self { _sink:sink, player, duration: Duration::ZERO, play_list: playlist,
            tap: Arc::new(SpectrumBuffer::new(1.0, 192000)), sample_rate: 192000 ,
        playing_index:None,volume:1.0,current_path:None,device_lost})
    }
    pub fn modify_current(&mut self,index:usize){
        self.player.stop();
        self.player.clear();
        self.play_list.lock().unwrap().current_index = Some(index);
    }

    pub fn update_playing_index(&mut self,lib:&MusicLibrary){
        let (current_index_option, queue_clone) = {
            let play_list = self.play_list.lock().unwrap();
            let current_index = play_list.current_index;
            let queue = play_list.queue.clone();
            (current_index, queue)
        }; 
        if let Some(current_playlist_index) = current_index_option{
            let current_id = lib.id_by_index[&queue_clone[current_playlist_index]];
            let target_index = lib.index_by_id[&current_id]; //在lib里的index
            if let Some((playlist_index,_)) = queue_clone.par_iter()
                .enumerate()
                .find_any(|(_ ,lib_index)|lib_index == &&target_index){
                    self.playing_index = Some(playlist_index);
                }
        }
    }
    pub fn next_music(&mut self){
        let mut play_list = self.play_list.lock().unwrap();
        match play_list.current_index{
            Some(index) =>{
                if index + 1 >= play_list.queue.len(){
                    play_list.current_index = Some(0);
                }else{
                    play_list.current_index = Some(index+1)
                }
            }
            None => {}
        }
    }
    pub fn previous_music(&mut self){
        let mut play_list = self.play_list.lock().unwrap();
        match play_list.current_index{
            Some(index) =>{
                if index < 1 {
                    play_list.current_index = Some(play_list.queue.len()-1);
                }else{
                    play_list.current_index = Some(index-1)
                }
            }
            None => {}
        }
    }
    //停止所有播放，清空player内置播放队列，开始播放当前音乐
    pub async fn play(&mut self,lib:&MusicLibrary) -> Result<()>{
        self.player.stop();
        self.player.clear();
        self.duration = Duration::ZERO;
        let target = {
            let play_list = self.play_list.lock().unwrap();
            match play_list.current_index{
                Some(index) =>{
                    let dur = Duration::from_secs(lib.tracks[play_list.queue[index]].duration as u64);
                    let path:Arc<str> = lib.tracks[play_list.queue[index]].path.clone();
                    Some((path, dur, index))
                }
                None => None,
            }
        };
        match target{
            Some((path, dur, index)) =>{
                self.duration = dur;
                self.playing_index = Some(index);
                self.current_path = Some((path.clone(), dur));
                self.load_path(path).await?;
            }
            None =>{
                tracing::warn!("尝试在play_list.current_index为None时访问");
            }
        }
        Ok(())
    }

    //解码 + 预取 + 频谱取流 + 送入引擎并开始播放（供 play 与设备重建复用）
    async fn load_path(&mut self, path: Arc<str>) -> Result<()> {
        let tap = self.tap.clone();
        let source = tokio::task::spawn_blocking(move || {
            let file = File::open(path.as_ref())?;
            let src = Decoder::try_from(file)?;
            let sr = src.sample_rate().get(); // 真实采样率
            let prefetched = PrefetchSource::new(src, 3); // 3 秒预取
            let tapper = TapSource::new(prefetched, tap); // 频谱取流
            Ok::<_, color_eyre::Report>((tapper, sr))
        })
        .await??;
        self.sample_rate = source.1;
        self.player.append(source.0);
        self.player.play();
        Ok(())
    }

    pub fn pause(&self){ 
        self.player.pause();
    }
    pub fn resume(&self){
        if self.player.empty() {return;}
        self.player.play();
    }
    pub fn is_playing(&self) -> bool {
        //not paused and queue is not empty
        !self.player.is_paused() && !self.player.empty()
    }
    pub fn is_paused(&self) -> bool {
        //paused and queue is not empty
        self.player.is_paused() && !self.player.empty()
    }
    pub fn stop(&self) {
        self.player.stop();
        self.player.clear();
    }
    //获取进度
    pub fn position(&self) -> Duration {
        self.player.get_pos()
    }
    //获取总时长
    pub fn duration(&self) -> Duration {
        self.duration
    }
    //频谱共享缓冲
    pub fn tap(&self) -> SharedSpectrum {
        self.tap.clone()
    }
    //当前曲目的真实采样率
    pub fn spectrum_sample_rate(&self) -> u32 {
        self.sample_rate
    }
    //进度百分比
    pub fn progress(&self) -> f64 {
        if self.duration.is_zero() {
            0.0
        } else {
            self.position().as_secs_f64() / self.duration.as_secs_f64()
        }
    }
    //快进/快退：相对当前进度跳转
    pub fn seek_forward(&self, secs: u64) {
        let target = self.position() + Duration::from_secs(secs);
        self.seek(target);
    }
    pub fn seek_backward(&self, secs: u64) {
        let target = self
            .position()
            .checked_sub(Duration::from_secs(secs))
            .unwrap_or_default();
        self.seek(target);
    }
    //跳到指定位置，并约束在总时长内
    pub fn seek(&self, target: Duration) {
        let target = target.min(self.duration);
        if self.player.try_seek(target).is_err() {
            tracing::warn!("当前曲目不支持 seek，已忽略跳转");
        }
    }
    //上一帧在播放，下一帧停了，说明一曲播放结束
    pub fn track_ended(&self, previously_playing: bool) -> bool {
        
        previously_playing && !self.is_paused() && self.player.empty()
    }

    pub fn volume_up(&mut self){
        let new = (self.volume + 0.01).clamp(0.0, 1.0);
        self.player.set_volume(new);
        self.volume = new
    }
    pub fn volume_down(&mut self){
        let new = (self.volume - 0.01).clamp(0.0, 1.0);
        self.player.set_volume(new);
        self.volume = new
    }
    pub fn volume(&self) -> f32{
        self.volume
    }
    /// 返回正在播放歌曲的歌词文件路径
    pub fn get_current_lyric_path(&self, lib: &MusicLibrary) -> Option<Arc<str>> {
        if let Some(index) = self.playing_index {
            let play_list = self.play_list.lock().unwrap();
            if let Some(song) = lib.tracks.get(play_list.queue[index]) {
                return song.lyric_path.clone();
            }
        }
        None
    }
    pub fn get_playing_info<'a>(&'a self,lib:&'a MusicLibrary) ->Option<(Cow<'a,str>,Cow<'a,str>)>{
        if let Some(index) = self.playing_index{
            let play_list = self.play_list.lock().unwrap();
            match lib.tracks.get(play_list.queue[index]){
                Some(song) => {
                    let (title,artist) = (&song.title,&song.artist);

                    Some((title.into(),match artist.as_ref(){
                        Some(a) =>a.into(),
                        None =>"".into()
                    }))
                }
                None => Some(("".into(),"".into()))
            }   
        }else{None}
    }

    /// 返回正在播放歌曲的采样率(Hz)与位深(bit)
    pub fn get_playing_quality(&self, lib: &MusicLibrary) -> Option<(Option<u32>, Option<u32>)> {
        if let Some(index) = self.playing_index {
            let play_list = self.play_list.lock().unwrap();
            if let Some(song) = lib.tracks.get(play_list.queue[index]) {
                return Some((song.sample_rate, song.bit_depth));
            }
        }
        None
    }

    //查询当前系统默认输出设备名，用于检测设备热切换
    pub fn current_device_name() -> Option<String> {
        rodio::cpal::default_host()
            .default_output_device()
            .and_then(|d| d.description().ok().map(|desc| desc.name().to_string()))
    }

    //查询当前系统默认输出设备的稳定唯一 ID
    pub fn current_device_id() -> Option<String> {
        rodio::cpal::default_host()
            .default_output_device()
            .and_then(|d| d.id().ok().map(|id| id.to_string()))
    }

    //设备是否已丢失（由流错误回调置位）
    pub fn device_lost(&self) -> bool {
        self.device_lost.load(Ordering::SeqCst)
    }

    //当系统默认输出设备变化或旧设备丢失时重建音频流，并恢复播放进度与状态
    pub async fn rebuild_device(&mut self) -> Result<()> {
        let was_playing = self.is_playing();
        let had_audio = !self.player.empty();
        let pos = self.player.get_pos();
        let saved = self.current_path.clone();

        let sink = Self::build_sink(self.device_lost.clone())?;
        let engine = Player::connect_new(sink.mixer());
        self._sink = sink;
        self.player = engine;
        self.device_lost.store(false, Ordering::SeqCst);
        if let Some(n) = Self::current_device_name() {
            tracing::info!("音频输出设备已切换为: {n}");
        }

        if had_audio {
            if let Some((path, dur)) = saved {
                self.duration = dur;
                self.load_path(path).await?;
                let _ = self.player.try_seek(pos.min(dur));
                if !was_playing {
                    self.player.pause();
                }
            }
        }
        Ok(())
    }
}
impl App{
    //处理一首音乐播放完
    pub async fn handle_track_end(&mut self){
        match self.repeatmode{
            RepeatMode::Sequential =>{
                // 只在判断期间短暂持锁，避免持锁时再调用 next_music（同一把锁）导致死锁
                let has_next = {
                    let q = self.play_queue.lock().unwrap();
                    match q.current_index {
                        Some(i) => i + 1 < q.queue.len(),
                        None => false,
                    }
                };
                if has_next {
                    self.player.next_music();
                    self.player.play(&self.music_list).await.unwrap();
                    self.was_playing = true;
                } else {
                    self.player.stop();
                    self.was_playing = false;
                }
            }
            RepeatMode::Random =>{
                let len = self.play_queue.lock().unwrap().queue.len();
                if len == 0 {
                    self.was_playing = false;
                    return;
                }
                let mut rng = rand::rng();
                let idx = rng.random_range(0..len);
                self.play_queue.lock().unwrap().current_index = Some(idx);
                self.player.play(&self.music_list).await.unwrap();
                self.was_playing = true;
            }
            RepeatMode::All =>{
                self.player.next_music();
                self.player.play(&self.music_list).await.unwrap();
                self.was_playing = true;
            }
            RepeatMode::One =>{
                self.player.play(&self.music_list).await.unwrap();
                self.was_playing = true;
            }
        }
    }
}
use std::collections::HashMap;
use std::io;
use std::path::{ Path, PathBuf};
use std::sync::{Arc,Mutex};
use color_eyre::eyre::{Ok, Result, eyre};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::Accessor;
use rfd::{AsyncFileDialog};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{SqlitePool,FromRow,Row};
use walkdir::WalkDir;
use crate::database::{self, add_meta, get_all_paths,  select_by_path, get_all_raw, update_lyric_path};
use crate::player::MyPlayer;
use crate::ui::AppUIState;
#[derive(PartialEq)]
pub enum AppMode{
    Main,
    Library,
}

pub enum RepeatMode {
    Sequential,
    Random,
    All,
    One,
}
impl RepeatMode{
    pub fn change(&self) -> Self{
        match self{
            Self::Sequential => Self::Random,
            Self::Random => Self::All,
            Self::All => Self::One,
            Self::One => Self::Sequential,
        }
    }
}
pub struct App{
    pub current_screen:AppMode,
    pub app_ui_state:AppUIState,
    pub repeatmode:RepeatMode,
    pub database:SqlitePool,
    pub player:MyPlayer,
    pub music_list:MusicLibrary,
    pub play_queue:Arc<Mutex<PlayQueue>>,
    pub was_playing:bool, //上一帧是否在播放，用于监测歌曲播放结束
    pub last_device_name:Option<String>,      //上次检测到的默认输出设备名
    pub last_device_id:Option<String>,        //上次检测到的默认输出设备稳定 ID
    pub last_device_check:std::time::Instant, //上次检测音频设备的时间
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Song {
    pub id:Option<i64>,
    pub title: String,
    pub artist:Option<String>,
    pub album:Option<String>,
    pub path: Arc<str>,
    pub duration: i64,//second
    pub in_list:i64,
    pub lyric_path:Option<Arc<str>>,
    pub sample_rate:Option<u32>, // 采样率（Hz）
    pub bit_depth:Option<u32>,   // 位深（bit）
}

impl<'r> FromRow<'r, SqliteRow> for Song {
    fn from_row(row: &'r SqliteRow) -> std::result::Result<Self, sqlx::Error> {
        let lyric_path: Option<String> = row.try_get("lyric_path")?;
        let path: String = row.try_get("path")?;
        Result::Ok(Song {
            id: row.try_get("id")?,
            title: row.try_get("title")?,
            artist: row.try_get("artist")?,
            album: row.try_get("album")?,
            path: Arc::from(path),
            duration: row.try_get("duration")?,
            in_list: row.try_get("in_list")?,
            lyric_path: lyric_path.map(|s| Arc::from(s)),
            sample_rate: row.try_get("sample_rate")?,
            bit_depth: row.try_get("bit_depth")?,
        })
    }
}
impl Song{
    pub async fn new<T:AsRef<Path>>(title:String,artist:Option<String>,album:Option<String>,path: T,lyric_path:Option<T>,duration:i64,sample_rate:Option<u32>,bit_depth:Option<u32>,db:&SqlitePool) -> Result<Self>{
        let path_str = path.as_ref().to_str().ok_or_else(||{
            tracing::warn!("文件路径包含非UTF‑8字符，无法存入数据库");
            eyre!("文件路径包含非UTF‑8字符，无法存入数据库")
        })?
        .to_string();
        let lyric_path = match lyric_path{
            Some(p) =>{
                match p.as_ref().to_str(){
                    Some(p) => Some(Arc::from(p)),
                    None => {
                        tracing::warn!("歌词文件包含非UTF‑8字符，无法存入数据库");
                        None
                    }
                }
            }
            None => None
        };

        let new_song = Song{ id: None, title:title, artist:artist, album:album, path: path_str.into(), duration:duration, in_list: 0,lyric_path:lyric_path, sample_rate, bit_depth };
        add_meta(db, &new_song).await?;
        select_by_path(db, &path).await?.ok_or_else(||{
            tracing::error!("Song(无id): {} 已经添加到了数据库却在select时错误",&path.as_ref().to_string_lossy());
            eyre!("Song(无id):已经添加到了数据库却在select时错误")
        })
    }
}
#[derive(Debug)]
pub struct MusicLibrary{
    pub tracks:Vec<Song>,
    pub id_by_index:std::collections::HashMap<usize,i64>,//index:id
    pub index_by_id:std::collections::HashMap<i64,usize>//id:index
}
impl MusicLibrary{
    pub fn new(tracks:Vec<Song>) ->Self{
        let length = tracks.len();
        let mut map= std::collections::HashMap::with_capacity(length);
        let mut map2 = std::collections::HashMap::with_capacity(length);
        for index in 0..length{
            if let Some(song) = tracks.get(index){
                map.insert(index,song.id.unwrap());
                map2.insert(song.id.unwrap(), index);
            }else{
                continue;
            }
        }
        Self{tracks:tracks,id_by_index:map,index_by_id:map2}
    }
    // pub fn get_in_list_index(&self) -> Vec<usize>{
    //     let mut result = Vec::new();
    //     for song in &self.tracks{
    //         if song.in_list == 1{
    //             if let Some(id) = song.id{
    //                 match self.index_by_id.get(&id) {
    //                     Some(index) => result.push(*index),
    //                     None => tracing::warn!("未从音乐库中根据id找到索引")
    //                 }
    //             }
    //         }
    //     }
    //     result
    // }
    // pub fn get_in_list_id(&self) -> Vec<i64>{
    //     let mut result = Vec::new();
    //     for song in &self.tracks{
    //         if song.in_list == 1{
    //             if let Some(id) = song.id{
    //                 result.push(id);
    //             }else{
    //                 tracing::error!("get_in_list_id时id为空");
    //             }
    //         }
    //     }
    //     result
    // }
}
#[derive(Debug)]
pub struct PlayQueue{
    pub queue: Vec<usize>,
    pub current_index: Option<usize>,
}
impl PlayQueue{
    pub fn new(lib:&MusicLibrary) -> Self{
        let mut queue =  Vec::new();
        for index in 0..lib.tracks.len(){
            if lib.tracks[index].in_list == 1{
                queue.push(index);
            }
        }
        Self { queue, current_index: None }
    }
    pub fn update(&mut self,lib:&MusicLibrary){
        // 记住当前歌曲 id，用于在重建后回溯其新位置
        let current_song_id = self.current_index
            .and_then(|i| self.queue.get(i))
            .and_then(|&track_index| lib.tracks.get(track_index))
            .and_then(|song| song.id);

        let mut queue =  Vec::new();
        for index in 0..lib.tracks.len(){
            if lib.tracks[index].in_list == 1{
                queue.push(index);
            }
        }
        self.queue = queue;

        // 若原当前歌曲仍在列表中，修正 current_index 到新位置；否则置 None
        let new_current = current_song_id.and_then(|id| {
            self.queue.iter().position(|&track_index| lib.tracks[track_index].id == Some(id))
        });
        self.current_index = new_current;
    }
}
impl App{
    pub async fn new() -> Result<Self>{
        let database = database::init_db().await?;
        let music_library = MusicLibrary::new(load_from_database(&database).await);
        let play_queue = Arc::new(Mutex::new(PlayQueue::new(&music_library)));
        let player = MyPlayer::try_new( play_queue.clone())?;
        let last_device_name = MyPlayer::current_device_name();
        let last_device_id = MyPlayer::current_device_id();
        let app = Self {
            current_screen:AppMode::Library,
            database,
            player:player,
            play_queue,
            music_list: music_library,
            app_ui_state:AppUIState::new(),
            repeatmode:RepeatMode::Sequential,
            was_playing:false,
            last_device_name,
            last_device_id,
            last_device_check:std::time::Instant::now(),
        };
        Ok(app)
    }
    pub async fn import_music(&mut self) ->Result<()>{
        self.music_list = MusicLibrary::new(import_music(&self.database).await?);
        Ok(())
    }
    //周期性检测系统默认输出设备是否变化，变化则重建音频流（热切换）
    pub async fn check_audio_device(&mut self){
        // 节流：约 1 秒检测一次，避免频繁枚举 WASAPI 设备
        if self.last_device_check.elapsed() < std::time::Duration::from_secs(1) {
            return;
        }
        self.last_device_check = std::time::Instant::now();

        let name = MyPlayer::current_device_name();
        let id = MyPlayer::current_device_id();
        let device_lost = self.player.device_lost();
        let name_changed = name.is_some() && name != self.last_device_name;
        let id_changed = id.is_some() && id != self.last_device_id;
        if device_lost || name_changed || id_changed {
            if let Err(e) = self.player.rebuild_device().await {
                tracing::error!("重建音频设备失败: {e}");
            }
            // 重建后重新读取默认设备名/ID，避免在过渡期间重复触发重建
            if let Some(n) = MyPlayer::current_device_name() {
                self.last_device_name = Some(n);
            }
            if let Some(i) = MyPlayer::current_device_id() {
                self.last_device_id = Some(i);
            }
        }
    }
    
}
//标题，艺术家，专辑，时长秒数，采样率，位深
async fn get_audio_info(path: impl AsRef<Path>) -> Result<(Option<String>,Option<String>,Option<String>,i64,Option<u32>,Option<u32>)>{
    let path = path.as_ref().to_owned();
    let res = tokio::task::spawn_blocking(move||{
        let tagged_file = lofty::read_from_path(path)?;
        let tag = tagged_file.primary_tag();
        let title = tag.and_then(|t|t.title().map(|s| s.to_string()));
        let artist = tag.and_then(|t| t.artist().map(|s|s.to_string()));
        let album = tag.and_then(|t| t.album().map(|s|s.to_string()));
        let props = tagged_file.properties();
        let duration_sec = props.duration().as_secs() as i64;
        let sample_rate = props.sample_rate();
        let bit_depth = props.bit_depth().map(|b| b as u32);
        Ok((title,artist,album,duration_sec,sample_rate,bit_depth))
    }).await??;
    Ok(res)
}
async fn clear_invalid_meta(database:&SqlitePool){
    if let Result::Ok(paths) = get_all_paths(database).await{
        for path in &paths{
            match tokio::fs::try_exists(path).await{
                io::Result::Ok(bool) if !bool =>{
                    match database::remove_by_path(database, path).await{
                    Err(e) => tracing::error!("在数据库中尝试删除无效的{path}失败: {e}"),
                    Result::Ok(sqlrep) =>tracing::info!("数据{path}无效，已从数据库中删除 {sqlrep:#?}"),
                    }
                },
                io::Result::Err(e) =>{
                    tracing::error!("清理无效数据时try_exists失败 {e}")
                },
                _ =>{}
            }  
        }
        
    }else{tracing::error!("在清理无效数据时sql查表失败");return;}
    
}
//查找歌词文件：优先音频同目录的同名 .lrc；否则在整个目录树的歌词索引中查找同名歌词
fn find_lrc_path(
    audio_path: impl AsRef<Path>,
    lrc_index: Option<&HashMap<String, PathBuf>>,
) -> Option<PathBuf> {
    let audio_path = audio_path.as_ref();
    let file_stem = audio_path.file_stem()?;
    // 1.优先同目录同名歌词
    if let Some(dir) = audio_path.parent() {
        let mut candidate = dir.join(file_stem);
        candidate.set_extension("lrc");
        if candidate.exists() && candidate.is_file() {
            return Some(candidate);
        }
    }
    // 2.同目录没有，从整棵目录树索引中查找同名歌词
    if let Some(index) = lrc_index {
        if let Some(stem) = file_stem.to_str() {
            if let Some(path) = index.get(stem) {
                return Some(path.clone());
            }
        }
    }
    None
}
///从数据库加载音乐信息
async fn load_from_database(db:&SqlitePool) -> Vec<Song>{
    clear_invalid_meta(db).await;
    get_all_raw(db).await.unwrap_or_else(|e|{
            tracing::error!("数据库读取失败 {e}");
            Vec::<Song>::new()
        })
}

///添加新的音乐
async fn import_music(db:&SqlitePool) -> Result<Vec<Song>>{
    clear_invalid_meta(db).await;
    const TARGET_EXTENSION:[&str;4] = ["flac","mp3","ogg","wav"];
    let default_music_dir = match dirs::audio_dir(){
        Some(a) => a,
        None => PathBuf::new(),
    };
    if let Some(folder) = AsyncFileDialog::new()
        .set_title("选择一个文件夹")
        .set_directory(default_music_dir)
        .pick_folder()
        .await{
            // 单次遍历：收集音频文件，并建立整棵目录树的歌词索引（文件名主干 -> 歌词路径）
            let mut audio_paths: Vec<PathBuf> = Vec::new();
            let mut lrc_index: HashMap<String, PathBuf> = HashMap::new();
            for entry in WalkDir::new(folder.path()
                .to_path_buf())
                .into_iter()
                .filter_map(|e| e.ok()){
                let path = entry.path().to_path_buf();
                if !path.is_file() {
                    continue;
                }
                let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                    continue;
                };
                if ext.eq_ignore_ascii_case("lrc") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        lrc_index.entry(stem.to_string()).or_insert(path);
                    }
                } else if TARGET_EXTENSION.contains(&ext) {
                    audio_paths.push(path);
                }
            }
            let mut handles = Vec::new();
            for path in audio_paths{
                // 查找歌词：优先同目录，其次整棵目录树索引
                let lyric_path = find_lrc_path(&path, Some(&lrc_index));
                let pool = db.clone();
                let handle = tokio::spawn(async move{
                    let (title,artist,album,duration_sec,sample_rate,bit_depth) = get_audio_info(&path).await?;
                    let title = title.unwrap_or_else(|| {
                            path.file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default()
                            //没有title用文件名代替
                    });
                    //检查是否存在
                    match select_by_path(&pool, &path).await{
                        Result::Ok(Some(song)) => {
                            // 已存在的歌曲：若数据库里缺少歌词、且本次在目录树中找到了，则回填歌词路径
                            if song.lyric_path.is_none() {
                                if let Some(lp) = lyric_path.as_deref() {
                                    if let Some(lp_str) = lp.to_str() {
                                        if let Some(id) = song.id {
                                            match update_lyric_path(&pool, id, lp_str).await {
                                                Result::Ok(_) => tracing::info!("已为 {} 回填歌词路径 {}", song.title, lp_str),
                                                Err(e) => tracing::error!("回填歌词路径失败: {e}"),
                                            }
                                        }
                                    }
                                }
                            }
                            tracing::info!("文件{}存在，已跳过",&path.to_string_lossy());
                            Ok(song)
                        }
                        Result::Ok(None) => {
                            let song = Song::new(title, artist, album, path.as_path(),lyric_path.as_deref(), duration_sec, sample_rate, bit_depth, &pool).await?;
                            tracing::info!("文件{}已添加到数据库",&path.to_string_lossy());
                            Ok(song)
                        }
                        Err(e) =>{
                            tracing::error!("在检查数据库中是否存在歌曲时出现错误 {e}");
                            Err(eyre!(e))
                        }
                    }                               
                });
                handles.push(handle);
            }
            let mut songs = Vec::new();
            for handle in handles{
                match handle.await{
                    Result::Ok(Result::Ok(song)) => songs.push(song),
                    Result::Ok(Err(e)) => tracing::warn!("处理文件失败: {}", e),
                    Err(e) => tracing::error!("任务被取消或 panic: {}", e),
                };
            }
            Ok(get_all_raw(db).await?)
    }else{
        tracing::warn!("文件夹未打开或打开失败，已跳过录入数据库，尝试读取已有的数据 ");
        let result = get_all_raw(db).await.unwrap_or_else(|e|{
            tracing::error!("数据库读取失败 {e}");
            Vec::<Song>::new()
        });
        Ok(result)
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    #[tokio::test]
    async fn test1(){
        let a= App::new().await.unwrap();
        println!("{:#?}",a.music_list);
    }
    #[test]
    fn test2(){
        assert_eq!(Path::new("D:\\games\\abc\\minecraft.exe").parent().unwrap(),Path::new("D:\\games\\abc"));
    }
}
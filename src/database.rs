use std::path::Path;
use crate::app::Song;
use color_eyre::Result;
use color_eyre::eyre::Ok;
use sqlx::Row;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteQueryResult;

pub async fn init_db() ->Result<SqlitePool>{
    let pool = SqlitePool::connect("sqlite://music_meta.db").await?;
    //建表
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS music_meta(
            id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            artist TEXT,
            album TEXT,
            path TEXT UNIQUE NOT NULL,
            duration INTEGER NOT NULL,
            in_list INTEGER NOT NULL,
            lyric_path TEXT,
            sample_rate INTEGER,
            bit_depth INTEGER
        )
        "#
    )
    .execute(&pool)
    .await?;
    // 兼容旧库：若列不存在则补齐（老库不会因为 CREATE TABLE IF NOT EXISTS 而新增列）
    ensure_column(&pool, "sample_rate", "INTEGER").await?;
    ensure_column(&pool, "bit_depth", "INTEGER").await?;
    Ok(pool)
}

/// 检查 music_meta 是否已有某列，没有则用 ALTER TABLE 追加。
async fn ensure_column(pool: &SqlitePool, name: &str, decl: &str) -> Result<()> {
    let rows = sqlx::query("PRAGMA table_info(music_meta)")
        .fetch_all(pool)
        .await?;
    let exists = rows.iter().any(|r| r.get::<String, _>("name") == name);
    if !exists {
        let sql = format!("ALTER TABLE music_meta ADD COLUMN {name} {decl}");
        sqlx::query(sql.as_str()).execute(pool).await?;
    }
    Ok(())
}
pub async fn add_meta(pool:&SqlitePool,meta:&Song) ->Result<SqliteQueryResult>{
    let path = meta.path.as_ref();
    let lyric_path = meta.lyric_path.as_deref();
    let result = sqlx::query(
        r#"INSERT OR IGNORE INTO music_meta (title, artist, album, path, lyric_path, duration, in_list, sample_rate, bit_depth) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"#
    )
    .bind(meta.title.as_str())
    .bind(meta.artist.as_deref())
    .bind(meta.album.as_deref())
    .bind(path)
    .bind(lyric_path)
    .bind(meta.duration)
    .bind(meta.in_list)
    .bind(meta.sample_rate)
    .bind(meta.bit_depth)
    .execute(pool)
    .await?;
    Ok(result)
}
// pub async fn remove_by_id(pool:&SqlitePool,id:i64) ->Result<SqliteQueryResult>{
//     let result = sqlx::query!(
//         r#"delete from music_meta where id = ?1"#,
//         id
//     )
//     .execute(pool)
//     .await?;
//     Ok(result)
// }
pub async fn remove_by_path(pool:&SqlitePool,path:impl AsRef<Path>) ->Result<SqliteQueryResult>{
    let path = path.as_ref().to_string_lossy();
    let result = sqlx::query!(
        r#"delete from music_meta where path = ?1"#,
        path
    )
    .execute(pool)
    .await?;
    Ok(result)

}
pub async fn get_all_raw(pool:&SqlitePool) ->Result<Vec<Song>>{
    let result = sqlx::query_as::<_, Song>(
        "SELECT id, title, artist, album, path, lyric_path, duration, in_list, sample_rate, bit_depth FROM music_meta"
    )
    .fetch_all(pool)
    .await?;
    Ok(result)
}
pub async fn select_by_path(pool: &SqlitePool, path: impl AsRef<Path>) -> Result<Option<Song>, sqlx::Error> {
    let path_str = path.as_ref().to_string_lossy();
    tracing::info!("查询 {}", path_str);
    let result = sqlx::query_as::<_, Song>(
        "SELECT id, title, artist, album, path, lyric_path, duration, in_list, sample_rate, bit_depth 
         FROM music_meta 
         WHERE path = ?1"
    )
    .bind(path_str.as_ref()) // 绑定 &str
    .fetch_optional(pool)
    .await?;
    std::result::Result::Ok(result)
}
// pub async fn select_by_id(pool:&SqlitePool,id:i64) ->Result<Option<Song>>{
//     let result = sqlx::query_as!(
//         Song,
//         r#"select id,title,artist,album,path,duration,in_list from music_meta where id = ?1"#,
//         id
//     )
//     .fetch_optional(pool)
//     .await?;
//     Ok(result)
// }
// pub async fn get_id_by_path(pool:&SqlitePool,path: impl AsRef<Path>) ->Result<Option<i64>>{
//     let path = path.as_ref().to_string_lossy();
//     let result = sqlx::query!(
//         r#"select id from music_meta where path = ?1"#,
//         path
//     )
//     .fetch_optional(pool)
//     .await?;
//     if let Some(record) = result{
//         Ok(Some(record.id))
//     }else{
//         Ok(None)
//     }
// }
pub async fn get_all_paths(pool:&SqlitePool) -> Result<Vec<String>>{
    let result = sqlx::query_scalar!(
        r#"select path from music_meta"#
    )
    .fetch_all(pool)
    .await?;
    Ok(result)
}
pub async fn modify_in_list_status_database(pool:&SqlitePool,id:i64,status:i64) -> Result<SqliteQueryResult>{
    let result = sqlx::query!(
        r#"
        update music_meta
        set in_list = ?1
        where id = ?2
        "#, status , id
    ).execute(pool).await?;
    Ok(result)
}
pub async fn update_lyric_path(pool:&SqlitePool,id:i64,lyric_path:&str) -> Result<SqliteQueryResult>{
    let result = sqlx::query(
        "UPDATE music_meta SET lyric_path = ?1 WHERE id = ?2"
    )
    .bind(lyric_path)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result)
}
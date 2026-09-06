-- Add migration script here
CREATE TABLE IF NOT EXISTS music_meta(
            id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            artist TEXT,
            album TEXT,
            path TEXT UNIQUE NOT NULL,
            lyric_path TEXT,
            duration INTEGER NOT NULL,
            in_list INTEGER NOT NULL,
            sample_rate INTEGER,
            bit_depth INTEGER

        )
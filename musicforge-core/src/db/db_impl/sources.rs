impl Db {
    /// 登记/更新媒体源，返回其 id。
    ///
    /// 幂等：同一路径重复登记不报错；`label` 为 `None` 时保留已有标签
    /// （重复扫描场景下不会把用户起的名字清掉）。
    pub fn upsert_source(&self, path: &str, label: Option<&str>) -> Result<i64, NcmError> {
        self.conn
            .execute(
                "INSERT INTO sources (path, label, enabled, added_at)
                 VALUES (?1, ?2, 1, unixepoch())
                 ON CONFLICT(path) DO UPDATE SET
                    label = COALESCE(excluded.label, sources.label)",
                params![path, label],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        self.conn
            .query_row("SELECT id FROM sources WHERE path = ?1", [path], |r| {
                r.get(0)
            })
            .map_err(|e| NcmError::Db(e.to_string()))
    }
    /// 列出全部媒体源（按添加时间序）。
    pub fn list_sources(&self) -> Result<Vec<SourceRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, path, label, enabled, added_at FROM sources ORDER BY added_at, id",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SourceRow {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    label: r.get(2)?,
                    enabled: r.get::<_, i64>(3)? != 0,
                    added_at: r.get(4)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }
    /// 移除媒体源并连带清理其曲目与行为行；返回被清理的曲目数。
    ///
    /// **FK 是活的**（P2 实测纠正）：rusqlite bundled 编译默认带
    /// `SQLITE_DEFAULT_FOREIGN_KEYS=1`——历史行存在时直接 DELETE tracks
    /// 会报 `FOREIGN KEY constraint failed`。因此清理必须按依赖顺序在
    /// **单事务**内完成：play_history / likes / playlist_items → tracks → sources。
    ///
    /// 语义：曲目行消失即行为行消失（重扫同一目录会产生新 track id，
    /// 旧行为本就无法再关联；留着只会成为孤儿）。
    /// 从资料库移除整个音源（级联清行为行）。`retain` 为 `true` 时**保留**
    /// `likes` / `play_history`（数据保留策略，见 [`crate::config::AppConfig::retain_likes_history`]），
    /// 仅清 `playlist_items` 与曲目/音源自身。
    pub fn remove_source(&self, id: i64, retain: bool) -> Result<usize, NcmError> {
        // 保留策略与活 FK 冲突：保留 likes/play_history 却删曲目会
        // `FOREIGN KEY constraint failed`。仅在此显式 opt-in 路径临时关闭 FK
        // （必须在事务**外**，SQLite 不允许事务内改 foreign_keys），操作后恢复。
        if retain {
            self.conn
                .execute("PRAGMA foreign_keys = OFF", [])
                .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        let outcome = (|| -> Result<usize, NcmError> {
            let tx = self
                .conn
                .unchecked_transaction()
                .map_err(|e| NcmError::Db(e.to_string()))?;
            if !retain {
                for sql in [
                    "DELETE FROM play_history WHERE track_id IN \
                     (SELECT id FROM tracks WHERE source_id = ?1)",
                    "DELETE FROM likes WHERE track_id IN \
                     (SELECT id FROM tracks WHERE source_id = ?1)",
                ] {
                    tx.execute(sql, [id])
                        .map_err(|e| NcmError::Db(e.to_string()))?;
                }
            }
            tx.execute(
                "DELETE FROM playlist_items WHERE track_id IN \
                 (SELECT id FROM tracks WHERE source_id = ?1)",
                [id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
            let n = tx
                .execute("DELETE FROM tracks WHERE source_id = ?1", [id])
                .map_err(|e| NcmError::Db(e.to_string()))?;
            tx.execute("DELETE FROM sources WHERE id = ?1", [id])
                .map_err(|e| NcmError::Db(e.to_string()))?;
            tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
            Ok(n)
        })();
        if retain {
            // B2：恢复 `foreign_keys` 必须**检查结果**——此前 `let _ =` 吞掉失败，
            // 且 SQLite 在活动事务内该 PRAGMA 为静默 no-op（不报错），两者叠加会让
            // 连接永久停在 FK=OFF（后续删除不再级联 → 孤儿行）。恢复失败必须上报，
            // 避免连接带病继续服务。
            if let Err(e) = self.conn.execute("PRAGMA foreign_keys = ON", []) {
                // 原操作已失败时保留其错误（诊断价值更高）；否则上报恢复失败。
                if outcome.is_ok() {
                    return Err(NcmError::Db(format!("failed to restore foreign_keys: {e}")));
                }
            }
        }
        outcome
    }
    /// 从资料库移除指定曲目（仅删索引行，不动文件）。FK 是活的（P2 实测），
    /// 必须由近及远在单事务内连带清理行为行，否则 `FOREIGN KEY constraint failed`。
    ///
    /// 返回被删除的曲目行数；空切片直接返回 0（不开启事务）。
    ///
    /// **SQLite 变量上限**：`ids` 可能远超 999（用户批量勾选上千首），`IN (?,..)`
    /// 占位符数 = `ids.len()` 会触发 `too many SQL variables`。按 [`MAX_SQL_VARS`]
    /// 拆批，每批一个独立 `IN` 子句，仍在**同一个事务**内（原子性不变）。
    pub fn remove_tracks(&self, ids: &[i64], retain: bool) -> Result<usize, NcmError> {
        if ids.is_empty() {
            return Ok(0);
        }
        // 保留策略与活 FK 冲突：保留 likes/play_history 却删曲目会
        // `FOREIGN KEY constraint failed`。仅在此显式 opt-in 路径临时关闭 FK
        // （必须在事务**外**），操作后恢复；默认路径（retain=false）FK 始终活。
        if retain {
            self.conn
                .execute("PRAGMA foreign_keys = OFF", [])
                .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        let outcome = (|| -> Result<usize, NcmError> {
            let tx = self
                .conn
                .unchecked_transaction()
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let mut deleted = 0usize;
            // retain = true → 跳过 likes / play_history，仅清 playlist_items
            let behavior_tables: &[&str] = if retain {
                &["playlist_items"]
            } else {
                &["play_history", "likes", "playlist_items"]
            };
            for chunk in ids.chunks(MAX_SQL_VARS) {
                let placeholders = vec!["?"; chunk.len()].join(",");
                for table in behavior_tables {
                    tx.execute(
                        &format!("DELETE FROM {table} WHERE track_id IN ({placeholders})"),
                        rusqlite::params_from_iter(chunk.iter()),
                    )
                    .map_err(|e| NcmError::Db(e.to_string()))?;
                }
                deleted += tx
                    .execute(
                        &format!("DELETE FROM tracks WHERE id IN ({placeholders})"),
                        rusqlite::params_from_iter(chunk.iter()),
                    )
                    .map_err(|e| NcmError::Db(e.to_string()))?;
            }
            tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
            Ok(deleted)
        })();
        if retain {
            // B2：恢复 `foreign_keys` 必须**检查结果**——此前 `let _ =` 吞掉失败，
            // 且 SQLite 在活动事务内该 PRAGMA 为静默 no-op（不报错），两者叠加会让
            // 连接永久停在 FK=OFF（后续删除不再级联 → 孤儿行）。恢复失败必须上报，
            // 避免连接带病继续服务。
            if let Err(e) = self.conn.execute("PRAGMA foreign_keys = ON", []) {
                // 原操作已失败时保留其错误（诊断价值更高）；否则上报恢复失败。
                if outcome.is_ok() {
                    return Err(NcmError::Db(format!("failed to restore foreign_keys: {e}")));
                }
            }
        }
        outcome
    }
    /// 批量写入曲目（单事务）。
    ///
    /// - 对不存在的 artist/album 先建后引（`INSERT OR IGNORE` + 回读 id），
    ///   索引层只提供名字，不感知 id 分配；
    /// - 专辑年份为空时用本次输入补齐（`year IS NULL` 守卫，不覆盖已有值）；
    /// - 已存在的 path 全量 UPDATE（重扫时元数据变化即时反映）；
    /// - `indexed_at` 标记本次索引 run——run 结束后用
    ///   [`Db::remove_stale_tracks`] 清掉不在本次索引里的行。
    pub fn upsert_tracks_batch(
        &self,
        tracks: &[TrackInput],
        indexed_at: i64,
    ) -> Result<usize, NcmError> {
        if tracks.is_empty() {
            return Ok(0);
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        {
            let mut st_artist = tx
                .prepare("INSERT OR IGNORE INTO artists (name) VALUES (?1)")
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let mut st_artist_id = tx
                .prepare("SELECT id FROM artists WHERE name = ?1")
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let mut st_album = tx
                .prepare(
                    "INSERT OR IGNORE INTO albums (title, album_artist_id, year)
                     VALUES (?1, ?2, ?3)",
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let mut st_album_id = tx
                .prepare(
                    "SELECT id FROM albums
                     WHERE title = ?1 AND IFNULL(album_artist_id, 0) = IFNULL(?2, 0)",
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let mut st_album_year = tx
                .prepare("UPDATE albums SET year = ?1 WHERE id = ?2 AND year IS NULL")
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let mut st_track = tx
                .prepare(
                    "INSERT INTO tracks (source_id, path, size, mtime, title, artist_id,
                                         album_id, track_no, disc_no, duration_ms, format,
                                         sample_rate, bit_depth, channels, is_lossless, indexed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                     ON CONFLICT(path) DO UPDATE SET
                        source_id=excluded.source_id, size=excluded.size, mtime=excluded.mtime,
                        title=excluded.title, artist_id=excluded.artist_id,
                        album_id=excluded.album_id, track_no=excluded.track_no,
                        disc_no=excluded.disc_no, duration_ms=excluded.duration_ms,
                        format=excluded.format, sample_rate=excluded.sample_rate,
                        bit_depth=excluded.bit_depth, channels=excluded.channels,
                        is_lossless=excluded.is_lossless, indexed_at=excluded.indexed_at",
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;

            for t in tracks {
                let artist_id = match non_empty(t.artist.as_deref()) {
                    Some(n) => {
                        st_artist
                            .execute(params![n])
                            .map_err(|e| NcmError::Db(e.to_string()))?;
                        let id: i64 = st_artist_id
                            .query_row(params![n], |r| r.get(0))
                            .map_err(|e| NcmError::Db(e.to_string()))?;
                        Some(id)
                    }
                    None => None,
                };
                // album_artist 缺省时回退到 track artist（专辑归属的最合理猜测）
                let album_artist_id = match non_empty(t.album_artist.as_deref()) {
                    Some(n) => {
                        st_artist
                            .execute(params![n])
                            .map_err(|e| NcmError::Db(e.to_string()))?;
                        let id: i64 = st_artist_id
                            .query_row(params![n], |r| r.get(0))
                            .map_err(|e| NcmError::Db(e.to_string()))?;
                        Some(id)
                    }
                    None => artist_id,
                };
                let album_id = match non_empty(t.album.as_deref()) {
                    Some(n) => {
                        st_album
                            .execute(params![n, album_artist_id, t.year])
                            .map_err(|e| NcmError::Db(e.to_string()))?;
                        let id: i64 = st_album_id
                            .query_row(params![n, album_artist_id], |r| r.get(0))
                            .map_err(|e| NcmError::Db(e.to_string()))?;
                        if let Some(y) = t.year {
                            // 补空不覆盖；失败忽略（缓存层降级哲学）
                            let _ = st_album_year.execute(params![y, id]);
                        }
                        Some(id)
                    }
                    None => None,
                };
                st_track
                    .execute(params![
                        t.source_id,
                        &t.path,
                        t.size,
                        t.mtime,
                        &t.title,
                        artist_id,
                        album_id,
                        t.track_no,
                        t.disc_no,
                        t.duration_ms,
                        &t.format,
                        t.sample_rate,
                        t.bit_depth,
                        t.channels,
                        t.is_lossless as i64,
                        indexed_at
                    ])
                    .map_err(|e| NcmError::Db(e.to_string()))?;
            }
        }
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(tracks.len())
    }
    /// 清理某媒体源下**不在本次索引 run 中**的曲目行（文件已删除/移走），
    /// 连带清理其行为行（FK 依赖顺序与 [`Db::remove_source`] 相同）。
    ///
    /// 依赖 `indexed_at < run_id` 判定，而不是把本次全部 path 传进 NOT IN——
    /// 十万级 path 列表会撞 SQLite 变量上限（默认 999），也白白放大 SQL。
    ///
    /// **P9 审计修复**：`retain = true` 时**不删** `likes` / `play_history`
    /// （用户不可再生数据），与 [`Db::remove_source`] / [`Db::remove_tracks`]
    /// 的保留策略完全一致。原实现无条件删除二者，与 `retain_likes_history`
    /// 默认 `true` 直接冲突——重扫索引时会把用户数据「静默清理」掉。
    /// `playlist_items` 是结构性关联（曲目没了则条目必悬空），两种模式都清。
    pub fn remove_stale_tracks(
        &self,
        source_id: i64,
        run_id: i64,
        retain: bool,
    ) -> Result<usize, NcmError> {
        // 保留策略与活 FK 冲突：保留 likes/play_history 却删曲目会
        // `FOREIGN KEY constraint failed`。仅在此显式 opt-in 路径临时关闭 FK
        // （必须在事务**外**），操作后恢复（与 remove_tracks 同法）。
        if retain {
            self.conn
                .execute("PRAGMA foreign_keys = OFF", [])
                .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        let outcome = (|| -> Result<usize, NcmError> {
            let tx = self
                .conn
                .unchecked_transaction()
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let behavior_tables: &[&str] = if retain {
                &["playlist_items"]
            } else {
                &["play_history", "likes", "playlist_items"]
            };
            for table in behavior_tables {
                tx.execute(
                    &format!(
                        "DELETE FROM {table} WHERE track_id IN \
                         (SELECT id FROM tracks WHERE source_id = ?1 AND indexed_at < ?2)"
                    ),
                    params![source_id, run_id],
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;
            }
            let n = tx
                .execute(
                    "DELETE FROM tracks WHERE source_id = ?1 AND indexed_at < ?2",
                    params![source_id, run_id],
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;
            tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
            Ok(n)
        })();
        if retain {
            // B2：恢复 `foreign_keys` 必须**检查结果**——此前 `let _ =` 吞掉失败，
            // 且 SQLite 在活动事务内该 PRAGMA 为静默 no-op（不报错），两者叠加会让
            // 连接永久停在 FK=OFF（后续删除不再级联 → 孤儿行）。恢复失败必须上报，
            // 避免连接带病继续服务。
            if let Err(e) = self.conn.execute("PRAGMA foreign_keys = ON", []) {
                // 原操作已失败时保留其错误（诊断价值更高）；否则上报恢复失败。
                if outcome.is_ok() {
                    return Err(NcmError::Db(format!("failed to restore foreign_keys: {e}")));
                }
            }
        }
        outcome
    }
    /// 各媒体源的曲目计数（`source_id → count`；源管理页展示用）。
    pub fn source_track_counts(
        &self,
    ) -> Result<std::collections::HashMap<i64, i64>, NcmError> {
        let mut stmt = self
            .conn
            .prepare("SELECT source_id, COUNT(1) FROM tracks GROUP BY source_id")
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let map = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<std::collections::HashMap<i64, i64>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(map)
    }
    /// 曲目总数。
    pub fn count_tracks(&self) -> Result<i64, NcmError> {
        self.conn
            .query_row("SELECT COUNT(1) FROM tracks", [], |r| r.get(0))
            .map_err(|e| NcmError::Db(e.to_string()))
    }
}

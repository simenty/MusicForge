impl Db {
    /// 曲库总览统计（艺术家/专辑按**实际关联的**曲目去重计数）。
    pub fn library_stats(&self) -> Result<LibraryStats, NcmError> {
        let (tracks, total_size, total_duration_ms) = self
            .conn
            .query_row(
                "SELECT COUNT(1), COALESCE(SUM(size), 0), COALESCE(SUM(duration_ms), 0) FROM tracks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let artists: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT artist_id) FROM tracks WHERE artist_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let albums: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT album_id) FROM tracks WHERE album_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(LibraryStats {
            tracks,
            artists,
            albums,
            total_size,
            total_duration_ms,
        })
    }
    /// 最常播放的曲目（按播放次数降序；同次数按标题稳定排序）。
    pub fn top_tracks(&self, limit: i64) -> Result<Vec<TopTrackRow>, NcmError> {
        let limit = limit.clamp(1, 100);
        // 列序 0..13 与 map_track_row 对齐，14 为聚合列
        let mut stmt = self
            .conn
            .prepare(
                "SELECT t.id, t.source_id, t.path, t.size, t.title, ar.name, al.title, \
                 t.track_no, t.duration_ms, t.format, t.sample_rate, t.bit_depth, t.channels, \
                 t.is_lossless, COUNT(h.id) AS plays \
                 FROM play_history h \
                 JOIN tracks t ON t.id = h.track_id \
                 LEFT JOIN artists ar ON ar.id = t.artist_id \
                 LEFT JOIN albums  al ON al.id = t.album_id \
                 GROUP BY t.id \
                 ORDER BY plays DESC, t.title \
                 LIMIT ?1",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([limit], |r| {
                Ok(TopTrackRow {
                    track: map_track_row(r)?,
                    play_count: r.get(14)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }
    /// 播放总览：`(总播放次数, 去重曲目数)`。
    /// 播放历史总量：`(总播放次数, 去重曲目数)`。
    ///
    /// **计数口径（审计修复）**：只统计仍能关联到现存曲目的播放记录——
    /// 与 [`Db::liked_count`] 同理，孤儿记录（曲目已删、FK 关闭或「永不删除
    /// 播放历史」策略下残留）不计入，避免统计数虚高、与可导航的历史视图脱节。
    pub fn history_totals(&self) -> Result<(i64, i64), NcmError> {
        self.conn
            .query_row(
                "SELECT COUNT(1), COUNT(DISTINCT track_id) FROM play_history ph \
                 WHERE EXISTS (SELECT 1 FROM tracks t WHERE t.id = ph.track_id)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| NcmError::Db(e.to_string()))
    }
    /// 按天播放计数（`since_secs` 之后；`day` 为**本地时区** `YYYY-MM-DD`，按日升序）。
    ///
    /// 时区交给 SQLite 的 `localtime` 修饰符——与前端分组（HistoryPage 的
    /// `new Date()` 本地时区）保持同一口径，避免跨 midnight 的双标。
    pub fn daily_play_counts(&self, since_secs: i64) -> Result<Vec<(String, i64)>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT date(played_at, 'unixepoch', 'localtime') AS day, COUNT(1) \
                 FROM play_history ph \
                 WHERE played_at >= ?1 \
                   AND EXISTS (SELECT 1 FROM tracks t WHERE t.id = ph.track_id) \
                 GROUP BY day ORDER BY day",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([since_secs], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }
}

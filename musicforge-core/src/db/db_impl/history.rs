impl Db {
    /// 追加一条播放记录（**追加日志**，不做去重——同一曲目多次播放就是多行）。
    pub fn record_play(
        &self,
        track_id: i64,
        played_at: i64,
        ms_played: i64,
    ) -> Result<(), NcmError> {
        self.conn
            .execute(
                "INSERT INTO play_history (track_id, played_at, ms_played) VALUES (?1, ?2, ?3)",
                params![track_id, played_at, ms_played],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }
    /// 播放历史（P6.21 排序 + P6.25 文本过滤；`Default` = 播放时间倒序；含曲目信息）。
    pub fn list_history_with(
        &self,
        sort: TrackSort,
        limit: i64,
        query: Option<&str>,
    ) -> Result<Vec<HistoryRow>, NcmError> {
        let limit = limit.clamp(1, 500);
        let (join, order_raw) = sort.clause();
        // `LikedAt` 需要 likes JOIN（本查询未 JOIN），回退默认序。
        let order = match (sort, order_raw.is_empty()) {
            (TrackSort::LikedAt, _) | (_, true) => "h.played_at DESC, h.id DESC",
            _ => order_raw,
        };
        let pred = query.and_then(|q| track_filter_pred(q, 2));
        let where_sql = match &pred {
            Some((sql, _)) => format!(" WHERE {sql}"),
            None => String::new(),
        };
        // 列序与 map_track_row 保持一一对应（0..13），14/15 为历史列。
        let mut binds: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(limit)];
        if let Some((_, pat)) = &pred {
            binds.push(Box::new(pat.clone()));
        }
        let mut stmt = self
            .conn
            .prepare(&format!(
                // 前 15 列必须与 `map_track_row` 对齐（含 t.style_code），聚合列在其后
                "SELECT t.id, t.source_id, t.path, t.size, t.title, ar.name, al.title, \
                 t.track_no, t.duration_ms, t.format, t.sample_rate, t.bit_depth, t.channels, \
                 t.is_lossless, t.style_code, h.played_at, h.ms_played \
                 FROM play_history h \
                 JOIN tracks t ON t.id = h.track_id \
                 LEFT JOIN artists ar ON ar.id = t.artist_id \
                 LEFT JOIN albums  al ON al.id = t.album_id \
                 {join} {where_sql} ORDER BY {order} LIMIT ?1"
            ))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())),
                |r| {
                    Ok(HistoryRow {
                        track: map_track_row(r)?,
                        played_at: r.get(15)?,
                        ms_played: r.get(16)?,
                    })
                },
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| NcmError::Db(e.to_string()))?);
        }
        Ok(out)
    }
    /// 播放历史（P6.21：支持排序；`Default` = 播放时间倒序，不过滤）。
    pub fn list_history_sorted(
        &self,
        sort: TrackSort,
        limit: i64,
    ) -> Result<Vec<HistoryRow>, NcmError> {
        self.list_history_with(sort, limit, None)
    }
    /// 播放历史（按时间倒序；P6.21 委托给 [`list_history_sorted`]）。
    pub fn list_history(&self, limit: i64) -> Result<Vec<HistoryRow>, NcmError> {
        self.list_history_sorted(TrackSort::Default, limit)
    }
    /// 播放历史总数（与 [`Db::list_history_with`] 同过滤条件；P1-12）。
    ///
    /// 前端历史列表是**虚拟化的**，行数必须映射到历史结果集；此前却借用了
    /// [`Db::count_tracks_filtered`]（统计实体是 `tracks`）→ 行数错位、虚拟化
    /// 出现越界占位行。此处统计 `play_history` 行，并与列表共用同一
    /// `track_filter_pred`，使「列表 + 计数」自洽。
    pub fn count_history_filtered(&self, query: Option<&str>) -> Result<i64, NcmError> {
        let pred = query.and_then(|q| track_filter_pred(q, 1));
        let n = match &pred {
            Some((sql, pat)) => {
                let full = format!(
                    "SELECT COUNT(1) FROM play_history h \
                     JOIN tracks t ON t.id = h.track_id \
                     LEFT JOIN artists ar ON ar.id = t.artist_id \
                     LEFT JOIN albums al ON al.id = t.album_id \
                     WHERE {sql}"
                );
                let mut stmt = self
                    .conn
                    .prepare(&full)
                    .map_err(|e| NcmError::Db(e.to_string()))?;
                stmt.query_row([pat], |r| r.get::<_, i64>(0))
                    .map_err(|e| NcmError::Db(e.to_string()))?
            }
            // B3：无过滤分支必须与列表（list_history_with）**同源**——列表是
            // `JOIN tracks` 的 INNER JOIN，永不返回孤儿行；此前此处统计全表
            // （含孤儿）→ 计数 > 实际可取行数，虚拟化列表尾部越界 / 空占位
            // （正是本函数注释要防的缺陷）。孤儿行在 retain=true 删曲目后必然产生。
            None => self
                .conn
                .query_row(
                    "SELECT COUNT(1) FROM play_history h \
                     JOIN tracks t ON t.id = h.track_id",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(|e| NcmError::Db(e.to_string()))?,
        };
        Ok(n)
    }
    /// 清空播放历史；返回被清空的行数。
    pub fn clear_history(&self) -> Result<usize, NcmError> {
        self.conn
            .execute("DELETE FROM play_history", [])
            .map_err(|e| NcmError::Db(e.to_string()))
    }
    /// 最近播放（**按曲目去重**，取每首曲目的最近一次；首页「继续聆听」）。
    pub fn recent_tracks(&self, limit: i64) -> Result<Vec<TrackRow>, NcmError> {
        let limit = limit.clamp(1, 100);
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{TRACK_SELECT}
                 JOIN (SELECT track_id, MAX(played_at) AS last_at
                       FROM play_history GROUP BY track_id) h
                   ON h.track_id = t.id
                 ORDER BY h.last_at DESC
                 LIMIT ?1"
            ))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([limit], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }
}

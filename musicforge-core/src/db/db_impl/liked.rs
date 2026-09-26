impl Db {
    /// 设置/取消「喜欢」（幂等——重复设置同状态不产生副作用）。
    pub fn set_like(&self, track_id: i64, liked: bool) -> Result<(), NcmError> {
        if liked {
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO likes (track_id, created_at) VALUES (?1, unixepoch())",
                    [track_id],
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;
        } else {
            self.conn
                .execute("DELETE FROM likes WHERE track_id = ?1", [track_id])
                .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        Ok(())
    }
    /// 切换「喜欢」，返回切换后的状态（true = 已喜欢）。
    pub fn toggle_like(&self, track_id: i64) -> Result<bool, NcmError> {
        let liked = self.is_liked(track_id)?;
        self.set_like(track_id, !liked)?;
        Ok(!liked)
    }
    /// 批量取消喜欢（仅删 `likes` 行；不动曲目/文件）。返回被取消的条数；
    /// 空切片直接返回 0（不开启事务）。P6.23 收藏页批量清理用。
    ///
    /// **SQLite 变量上限**：`ids` 可能远超 999（收藏页批量勾选上千首），
    /// 按 [`MAX_SQL_VARS`] 拆批执行，避免 `too many SQL variables`。
    pub fn unlike_tracks(&self, ids: &[i64]) -> Result<usize, NcmError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut total = 0usize;
        for chunk in ids.chunks(MAX_SQL_VARS) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            total += self
                .conn
                .execute(
                    &format!("DELETE FROM likes WHERE track_id IN ({placeholders})"),
                    rusqlite::params_from_iter(chunk.iter()),
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        Ok(total)
    }
    /// 是否已喜欢。
    pub fn is_liked(&self, track_id: i64) -> Result<bool, NcmError> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(1) FROM likes WHERE track_id = ?1",
                [track_id],
                |r| r.get(0),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(n > 0)
    }
    /// 喜欢列表（P6.21 排序 + P6.25 文本过滤）。
    /// 曲目已被移除的行自动消失——INNER JOIN。
    pub fn list_liked_with(
        &self,
        sort: TrackSort,
        limit: i64,
        offset: i64,
        query: Option<&str>,
    ) -> Result<Vec<TrackRow>, NcmError> {
        let limit = limit.clamp(1, 500);
        let offset = offset.max(0);
        let (join, order_raw) = sort.clause();
        // `PlayedAt` 需要 history JOIN（本查询未 JOIN），回退默认序。
        let order = match (sort, order_raw.is_empty()) {
            (TrackSort::PlayedAt, _) | (_, true) => "lk.created_at DESC, t.id DESC",
            _ => order_raw,
        };
        let pred = query.and_then(|q| track_filter_pred(q, 3));
        let where_sql = match &pred {
            Some((sql, _)) => format!(" WHERE {sql}"),
            None => String::new(),
        };
        let sql = format!(
            "{TRACK_SELECT}
             JOIN likes lk ON lk.track_id = t.id
             {join}
             {where_sql}
             ORDER BY {order}
             LIMIT ?1 OFFSET ?2"
        );
        let mut binds: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(limit), Box::new(offset)];
        if let Some((_, pat)) = &pred {
            binds.push(Box::new(pat.clone()));
        }
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())),
                map_track_row,
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| NcmError::Db(e.to_string()))?);
        }
        Ok(out)
    }
    /// 喜欢列表（P6.21：支持排序；`Default` = 收藏时间倒序，不过滤）。
    pub fn list_liked_sorted(
        &self,
        sort: TrackSort,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TrackRow>, NcmError> {
        self.list_liked_with(sort, limit, offset, None)
    }
    /// 喜欢列表（按收藏时间倒序；P6.21 委托给 [`list_liked_sorted`]）。
    pub fn list_liked(&self, limit: i64, offset: i64) -> Result<Vec<TrackRow>, NcmError> {
        self.list_liked_sorted(TrackSort::Default, limit, offset)
    }
    /// 喜欢总数。
    ///
    /// **计数口径（审计修复）**：只统计仍能关联到现存曲目的喜欢行——
    /// `list_liked_with` 用 `INNER JOIN tracks` 只展示存在曲目的喜欢，UI 计数
    /// 必须与列表自洽（孤儿行不计入）。曲目被移除时行为行由
    /// [`Db::remove_tracks`]/[`Db::remove_source`] 级联清理，但 FK 关闭或
    /// 「永不删除 likes」策略下可能残留孤儿，此处用 `EXISTS` 兜底过滤。
    pub fn liked_count(&self) -> Result<i64, NcmError> {
        self.conn
            .query_row(
                "SELECT COUNT(1) FROM likes lk \
                 WHERE EXISTS (SELECT 1 FROM tracks t WHERE t.id = lk.track_id)",
                [],
                |r| r.get(0),
            )
            .map_err(|e| NcmError::Db(e.to_string()))
    }
    /// 曲目总数（P6.25：`query` 非空时返回**过滤后**的计数——虚拟化列表的
    /// 行索引必须映射到过滤结果集，否则会出现越界占位行）。
    pub fn count_tracks_filtered(&self, query: Option<&str>) -> Result<i64, NcmError> {
        let pred = query.and_then(|q| track_filter_pred(q, 1));
        let n = match &pred {
            Some((sql, pat)) => {
                let full = format!(
                    "SELECT COUNT(1) FROM tracks t \
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
            None => self.count_tracks()?,
        };
        Ok(n)
    }
    /// 全部已喜欢的曲目 id（前端一次性拉取做行状态判定——避免逐行查询）。
    ///
    /// 刻意不分页：likes 是用户显式行为，量级远小于曲库本身；即便上万条，
    /// 一个 i64 数组的传输也远小于分页往返的成本。
    pub fn all_liked_ids(&self) -> Result<Vec<i64>, NcmError> {
        let mut stmt = self
            .conn
            .prepare("SELECT track_id FROM likes ORDER BY created_at DESC, track_id DESC")
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| r.get(0))
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<i64>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }
}

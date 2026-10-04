impl Db {
    /// 创建歌单；名称 trim 后不得为空。返回新 id。
    pub fn create_playlist(&self, name: &str) -> Result<i64, NcmError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(NcmError::Db("playlist name must not be empty".to_string()));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.conn
            .execute(
                "INSERT INTO playlists(name, created_at) VALUES (?1, ?2)",
                params![name, now],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(self.conn.last_insert_rowid())
    }
    /// 歌单列表（创建序；含曲目数）。
    pub fn list_playlists(&self) -> Result<Vec<PlaylistRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT p.id, p.name, \
                        COUNT(i.track_id) FILTER (WHERE EXISTS (SELECT 1 FROM tracks t WHERE t.id = i.track_id)) AS n \
                 FROM playlists p
                 LEFT JOIN playlist_items i ON i.playlist_id = p.id
                 GROUP BY p.id
                 ORDER BY p.created_at, p.id",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PlaylistRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    track_count: r.get(2)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }
    /// 歌单内曲目（按 position 顺序；显示名已解析）。
    /// INNER JOIN 语义：曲目行被移除（源删除）后条目自动隐藏。
    pub fn playlist_tracks(&self, playlist_id: i64) -> Result<Vec<TrackRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{TRACK_SELECT}
                 JOIN playlist_items pi ON pi.track_id = t.id AND pi.playlist_id = ?1
                 ORDER BY pi.position"
            ))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([playlist_id], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }
    /// 追加曲目到歌单：跳过已在歌单中的、跳过不存在的 track id；
    /// position 从尾部续接。返回实际追加数（单事务）。
    pub fn playlist_add_tracks(
        &self,
        playlist_id: i64,
        track_ids: &[i64],
    ) -> Result<usize, NcmError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let exists: i64 = tx
            .query_row(
                "SELECT COUNT(1) FROM playlists WHERE id = ?1",
                [playlist_id],
                |r| r.get(0),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        if exists == 0 {
            return Err(NcmError::Db(format!("playlist {playlist_id} does not exist")));
        }
        let mut have: std::collections::HashSet<i64> = {
            let mut stmt = tx
                .prepare("SELECT track_id FROM playlist_items WHERE playlist_id = ?1")
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let rows = stmt
                .query_map([playlist_id], |r| r.get::<_, i64>(0))
                .map_err(|e| NcmError::Db(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| NcmError::Db(e.to_string()))?;
            rows.into_iter().collect()
        };
        let mut pos: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_items WHERE playlist_id = ?1",
                [playlist_id],
                |r| r.get(0),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let mut added = 0usize;
        for &id in track_ids {
            if have.contains(&id) {
                continue;
            }
            // 曲目不存在则跳过（FK 是活的——静默跳过比报 FK 错更符合"批量加"语义）
            let ok: i64 = tx
                .query_row("SELECT COUNT(1) FROM tracks WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .map_err(|e| NcmError::Db(e.to_string()))?;
            if ok == 0 {
                continue;
            }
            tx.execute(
                "INSERT INTO playlist_items(playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                params![playlist_id, id, pos],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
            have.insert(id);
            pos += 1;
            added += 1;
        }
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(added)
    }
    /// 从歌单移除曲目（后续 position 前移，保持连续；单事务）。
    pub fn playlist_remove_track(&self, playlist_id: i64, track_id: i64) -> Result<(), NcmError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let pos: Option<i64> = tx
            .query_row(
                "SELECT position FROM playlist_items WHERE playlist_id = ?1 AND track_id = ?2",
                params![playlist_id, track_id],
                |r| r.get(0),
            )
            .ok();
        if let Some(p) = pos {
            tx.execute(
                "DELETE FROM playlist_items WHERE playlist_id = ?1 AND position = ?2",
                params![playlist_id, p],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
            tx.execute(
                "UPDATE playlist_items SET position = position - 1 \
                 WHERE playlist_id = ?1 AND position > ?2",
                params![playlist_id, p],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }
    /// 歌单内移动曲目到目标下标（拖拽排序；position 重排后保持连续 0..n-1）。
    /// 越界下标 clamp；曲目不在歌单 → 空操作。
    ///
    /// ⚠️ `playlist_items` 的 PK 是 `(playlist_id, position)`：逐行 ±1 的
    /// "中间段平移"会中途撞唯一约束（SQLite 逐行更新，顺序不保证）。
    /// 实现走**两阶段**：先把全部 position 抬到过渡区间（+100000），
    /// 再按新顺序逐行写回——无约束技巧依赖，长度上限远低于偏移量。
    pub fn playlist_move_track(
        &self,
        playlist_id: i64,
        track_id: i64,
        to_index: i64,
    ) -> Result<(), NcmError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        // ① 读全序（按 position）
        // ① 读全序（track_id + 原 position）
        // C1 修复：记录原 position，写回时用「抬区间后的唯一 position」定位，
        // 而非 track_id——歌单含重复 track_id（同一曲多行不同 position）时，用
        // track_id 定位会把多行一次性 SET 成同一 position，中途撞 PK 致整事务回滚。
        let mut items: Vec<(i64, i64)> = {
            let mut stmt = tx
                .prepare(
                    "SELECT track_id, position FROM playlist_items WHERE playlist_id = ?1 ORDER BY position",
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let rows = stmt
                .query_map([playlist_id], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
                })
                .map_err(|e| NcmError::Db(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| NcmError::Db(e.to_string()))?;
            rows
        };
        let Some(pos) = items.iter().position(|&(tid, _)| tid == track_id) else {
            return Ok(()); // 不在歌单 → 空操作
        };
        let to = to_index.clamp(0, (items.len() as i64 - 1).max(0)) as usize;
        if pos == to {
            return Ok(());
        }
        // ② 新顺序
        let moved = items.remove(pos);
        items.insert(to, moved);
        // ③ 抬到过渡区间（脱离 0..n-1，消除约束冲突）
        tx.execute(
            "UPDATE playlist_items SET position = position + 100000 WHERE playlist_id = ?1",
            [playlist_id],
        )
        .map_err(|e| NcmError::Db(e.to_string()))?;
        // ④ 按新顺序写回：用抬区间后的**唯一** position 定位（C1 修复）
        for (i, &(_, old_pos)) in items.iter().enumerate() {
            tx.execute(
                "UPDATE playlist_items SET position = ?1 WHERE playlist_id = ?2 AND position = ?3",
                params![i as i64, playlist_id, old_pos + 100000],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }
    /// 智能清理预览（P6.24）：返回 `(重复副本数, 失效条目数)`。
    ///
    /// - **重复副本**：同一 `track_id` 在歌单中出现多次时，除首条（最小 position）
    ///   之外的多余份数之和（`SUM(count - 1)`）。
    /// - **失效条目**：`track_id` 已不在 `tracks` 表中的条目（典型成因：重新索引
    ///   源目录会产生**新** track id，旧条目即成为孤儿；INNER JOIN 仍会隐藏它们，
    ///   但它们会虚增 `playlists.track_count`）。
    ///
    /// 只读，不修改数据。
    pub fn playlist_cleanup_preview(
        &self,
        playlist_id: i64,
    ) -> Result<(usize, usize), NcmError> {
        let dup_extra: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(c - 1), 0) FROM (
                   SELECT COUNT(*) AS c
                   FROM playlist_items
                   WHERE playlist_id = ?1
                   GROUP BY track_id
                   HAVING COUNT(*) > 1
                 )",
                [playlist_id],
                |r| r.get(0),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let orphans: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM playlist_items
                 WHERE playlist_id = ?1 AND track_id NOT IN (SELECT id FROM tracks)",
                [playlist_id],
                |r| r.get(0),
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok((dup_extra as usize, orphans as usize))
    }
    /// 智能清理执行（P6.24）：删除重复副本（保留每曲首条）+ 失效条目，
    /// 并在单事务内把剩余条目位置重排为连续 0..n-1。
    ///
    /// 返回 `(实际移除的重复副本数, 实际移除的失效条目数)`。
    pub fn playlist_smart_cleanup(
        &self,
        playlist_id: i64,
    ) -> Result<(usize, usize), NcmError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        // ① 重复副本：保留每 track_id 的最小 position，删除其余
        let removed_dups = tx
            .execute(
                "DELETE FROM playlist_items
                 WHERE playlist_id = ?1
                   AND position NOT IN (
                     SELECT MIN(position) FROM playlist_items
                     WHERE playlist_id = ?1 GROUP BY track_id
                   )
                   AND track_id IN (
                     SELECT track_id FROM playlist_items WHERE playlist_id = ?1
                     GROUP BY track_id HAVING COUNT(*) > 1
                   )",
                [playlist_id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        // ② 失效条目：track_id 已不在 tracks 表
        let removed_orphans = tx
            .execute(
                "DELETE FROM playlist_items
                 WHERE playlist_id = ?1
                   AND track_id NOT IN (SELECT id FROM tracks)",
                [playlist_id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        // ③ 重排位置为 0..n-1：先整体抬到安全高位（+10_000_000）避免 PK 冲突，
        //    再按 track_id（去重后唯一）逐行写回最终下标。
        let ids: Vec<i64> = {
            let mut stmt = tx
                .prepare(
                    "SELECT track_id FROM playlist_items WHERE playlist_id = ?1 ORDER BY position",
                )
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let rows = stmt
                .query_map([playlist_id], |r| r.get::<_, i64>(0))
                .map_err(|e| NcmError::Db(e.to_string()))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(|e| NcmError::Db(e.to_string()))?);
            }
            out
        };
        tx.execute(
            "UPDATE playlist_items SET position = position + 10000000 WHERE playlist_id = ?1",
            [playlist_id],
        )
        .map_err(|e| NcmError::Db(e.to_string()))?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE playlist_items SET position = ?1
                 WHERE playlist_id = ?2 AND track_id = ?3 AND position >= 10000000",
                params![i as i64, playlist_id, id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok((removed_dups, removed_orphans))
    }
    /// 重命名歌单（名称 trim 后不得为空）。
    pub fn playlist_rename(&self, playlist_id: i64, name: &str) -> Result<(), NcmError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(NcmError::Db("playlist name must not be empty".to_string()));
        }
        self.conn
            .execute(
                "UPDATE playlists SET name = ?1 WHERE id = ?2",
                params![name, playlist_id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }
    /// 删除歌单及其条目（FK 无级联——手动两条 DELETE，单事务；**不动曲目行**）。
    pub fn playlist_delete(&self, playlist_id: i64) -> Result<(), NcmError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        tx.execute(
            "DELETE FROM playlist_items WHERE playlist_id = ?1",
            [playlist_id],
        )
        .map_err(|e| NcmError::Db(e.to_string()))?;
        tx.execute("DELETE FROM playlists WHERE id = ?1", [playlist_id])
            .map_err(|e| NcmError::Db(e.to_string()))?;
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }
    /// 歌单封面候选（P6.12）：`(playlist_id, cover_path)`——按歌单内曲序，
    /// 每个歌单至多 `per` 条且路径去重（拼贴四格不应重复同图）。
    pub fn playlist_covers(&self, per: i64) -> Result<Vec<(i64, String)>, NcmError> {
        let per = per.clamp(1, 8) as usize;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT pi.playlist_id, al.cover_cache
                 FROM playlist_items pi
                 JOIN tracks t ON t.id = pi.track_id
                 JOIN albums al ON al.id = t.album_id
                 WHERE al.cover_cache IS NOT NULL
                 ORDER BY pi.playlist_id, pi.position",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        // 每单去重 + 截断（候选行本身很小，内存侧处理；SQL 窗口函数不值得）
        let mut out: Vec<(i64, String)> = Vec::new();
        let mut seen: std::collections::HashMap<i64, std::collections::HashSet<String>> =
            std::collections::HashMap::new();
        for (pid, path) in rows {
            let set = seen.entry(pid).or_default();
            if set.len() < per && set.insert(path.clone()) {
                out.push((pid, path));
            }
        }
        Ok(out)
    }
}

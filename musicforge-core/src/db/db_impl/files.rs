impl Db {
    /// 写入/更新一条文件索引（增量扫描的缓存依据）。
    pub fn upsert_file(
        &self,
        path: &str,
        size: i64,
        mtime: Option<i64>,
        format: Option<&str>,
        sha256: Option<&str>,
    ) -> Result<(), NcmError> {
        self.conn
            .execute(
                "INSERT INTO files (path, size, mtime, format, sha256, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
                 ON CONFLICT(path) DO UPDATE SET
                    size=excluded.size, mtime=excluded.mtime,
                    format=excluded.format, sha256=excluded.sha256,
                    updated_at=excluded.updated_at",
                params![path, size, mtime, format, sha256],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }
    /// 批量写文件索引（单事务；P1-10）。
    ///
    /// 把原本逐条 autocommit 的上万次写入合并为一次提交，全库刷新快一个数量级
    /// （审计：十万文件 = 十万次 autocommit）。每条幂等（`ON CONFLICT(path) DO
    /// UPDATE`），整批失败整体回滚后可安全重试（扫描本就按增量缓存重算）。
    pub fn upsert_files_batch(&self, rows: &[FileIndexRow]) -> Result<(), NcmError> {
        if rows.is_empty() {
            return Ok(());
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        for (path, size, mtime, format, sha256) in rows {
            tx.execute(
                "INSERT INTO files (path, size, mtime, format, sha256, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
                 ON CONFLICT(path) DO UPDATE SET
                    size=excluded.size, mtime=excluded.mtime,
                    format=excluded.format, sha256=excluded.sha256,
                    updated_at=excluded.updated_at",
                params![path, size, mtime, format, sha256],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        }
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }
    /// 读取一条文件索引。
    pub fn get_file(&self, path: &str) -> Result<Option<FileRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, size, mtime, format, sha256 FROM files WHERE path = ?1")
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let mut rows = stmt
            .query_map([path], |r| {
                Ok(FileRow {
                    path: r.get(0)?,
                    size: r.get(1)?,
                    mtime: r.get(2)?,
                    format: r.get(3)?,
                    sha256: r.get(4)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?;
        match rows.next() {
            Some(row) => Ok(Some(row.map_err(|e| NcmError::Db(e.to_string()))?)),
            None => Ok(None),
        }
    }
    /// D17 哈希缓存查询（L1+L2 语义）：
    ///
    /// 仅当缓存行的 `size` 与 `mtime` 都与当前一致时才返回哈希——
    /// 任一不一致（或行内无哈希）都返回 `None`，调用方需重算并回写。
    /// `mtime` 不可得的文件永远 miss（宁可重算，不返回过期哈希）。
    pub fn cached_hash(
        &self,
        path: &str,
        size: i64,
        mtime: i64,
    ) -> Result<Option<String>, NcmError> {
        let h: Option<String> = self
            .conn
            .query_row(
                "SELECT sha256 FROM files
                 WHERE path = ?1 AND size = ?2 AND mtime = ?3 AND sha256 IS NOT NULL",
                params![path, size, mtime],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(h)
    }
    /// 登记一次任务开始。
    pub fn start_task(&self, id: &str, command: &str, started_at: &str) -> Result<(), NcmError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO tasks (id, command, started_at, ok, failed)
                 VALUES (?1, ?2, ?3, 0, 0)",
                params![id, command, started_at],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }
    /// 标记一次任务结束。
    pub fn finish_task(
        &self,
        id: &str,
        finished_at: &str,
        ok: i64,
        failed: i64,
    ) -> Result<(), NcmError> {
        self.conn
            .execute(
                "UPDATE tasks SET finished_at=?2, ok=?3, failed=?4 WHERE id=?1",
                params![id, finished_at, ok, failed],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }
    /// 记录一次确认（如高风险插件的 acknowledge）。
    pub fn set_ack(&self, id: &str) -> Result<(), NcmError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO ack (id, at) VALUES (?1, datetime('now'))",
                params![id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }
    /// 是否已确认。
    pub fn has_ack(&self, id: &str) -> Result<bool, NcmError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(1) FROM ack WHERE id = ?1", [id], |r| r.get(0))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(n > 0)
    }
    /// 统计：`(files, tasks)`；用于诊断与测试。
    pub fn stats(&self) -> Result<(i64, i64), NcmError> {
        let files: i64 = self
            .conn
            .query_row("SELECT COUNT(1) FROM files", [], |r| r.get(0))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let tasks: i64 = self
            .conn
            .query_row("SELECT COUNT(1) FROM tasks", [], |r| r.get(0))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok((files, tasks))
    }
}

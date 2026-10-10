impl Db {
    /// 分页读取曲目（P6.21 排序 + P6.25 文本过滤）。
    ///
    /// `limit` 硬上限 500：IPC 层禁止全量序列化，分页是契约而非建议。
    /// `query` 为空/None = 不过滤；否则按 标题 / 艺术家 / 专辑 / 路径 模糊匹配
    /// （服务端过滤——虚拟化列表必须对**结果集**分页，不能前端切部分数据）。
    /// 同 [`Self::list_tracks_with`]，额外按**风格码**筛选（X15）。
    ///
    /// 刻意**新增方法**而非给 `list_tracks_with` 加参数：后者被大量既有测试与调用点
    /// 以 4 参形式使用，改签名会连带改动十余处断言（与本次功能无关的噪音）。
    pub fn list_tracks_filtered(
        &self,
        sort: TrackSort,
        limit: i64,
        offset: i64,
        query: Option<&str>,
        style_code: Option<&str>,
    ) -> Result<Vec<TrackRow>, NcmError> {
        let limit = limit.clamp(1, 500);
        let offset = offset.max(0);
        let (join, order_raw) = sort.clause();
        // `PlayedAt`/`LikedAt` 的 ORDER BY 引用 `h.`/`lk.` 别名，但本查询的
        // TRACK_SELECT 只 LEFT JOIN 了 ar/al，没 JOIN play_history/likes →
        // 直接下发会 prepare 失败（no such column）。与 list_liked_with /
        // list_history_with 同款回退到自然序，避免整条查询硬错。
        let order = match (sort, order_raw.is_empty()) {
            (TrackSort::PlayedAt, _) | (TrackSort::LikedAt, _) | (_, true) => "t.path",
            _ => order_raw,
        };
        // 两个谓词（文本 / 风格码）按序占位：?1=?limit ?2=?offset，谓词从 ?3 起。
        // ⚠️ 必须与 `count_tracks_filtered` 用**同一对 helper**，否则结果行数与行内容错位。
        let mut binds: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(limit), Box::new(offset)];
        let mut conds: Vec<String> = Vec::new();
        let mut next_idx = 3;
        if let Some(p) = query.and_then(|q| track_filter_pred(q, next_idx)) {
            next_idx += 1;
            binds.push(Box::new(p.1.clone()));
            conds.push(p.0);
        }
        if let Some(p) = style_code.and_then(|c| style_code_pred(c, next_idx)) {
            binds.push(Box::new(p.1.clone()));
            conds.push(p.0);
        }
        let where_sql = if conds.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conds.join(" AND "))
        };
        let sql = format!("{TRACK_SELECT} {join}{where_sql} ORDER BY {order} LIMIT ?1 OFFSET ?2");
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

    /// X15：聚合全部风格码 token 及其曲目数（供筛选下拉——用户不必先知道有哪些码）。
    ///
    /// 库里存的是规范化串 `-Y23-S01-E01-`，拆分在 **Rust 侧**做：先 `GROUP BY style_code`
    /// 拿到不同串（数量远小于行数），再拆 token 累加计数——比在 SQL 里拆字符串简单得多。
    ///
    /// 排序稳定：先按类别（Y/S/E/C/V，其余归末），再按码字典序——下拉里的顺序不随
    /// 增删曲目跳变。
    pub fn style_code_counts(&self) -> Result<Vec<(String, i64)>, NcmError> {
        let mut st = self
            .conn
            .prepare(
                "SELECT style_code, COUNT(1) FROM tracks
                 WHERE style_code IS NOT NULL AND style_code <> ''
                 GROUP BY style_code",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let mapped = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let grouped: Vec<(String, i64)> = mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;

        let mut acc = std::collections::BTreeMap::<String, i64>::new();
        for (s, n) in grouped {
            for tok in s.split('-') {
                if tok.is_empty() {
                    continue;
                }
                *acc.entry(tok.to_string()).or_default() += n;
            }
        }
        let mut out: Vec<(String, i64)> = acc.into_iter().collect();
        let rank = |code: &str| match code.chars().next() {
            Some('Y') => 0,
            Some('S') => 1,
            Some('E') => 2,
            Some('C') => 3,
            Some('V') => 4,
            _ => 5,
        };
        out.sort_by(|a, b| (rank(&a.0), &a.0).cmp(&(rank(&b.0), &b.0)));
        Ok(out)
    }

    /// X15：回填 `style_code`（**只处理 `IS NULL` 的行**）——升级到 v4 后，存量曲库
    /// 该列全为 NULL，若不回填就必须整库重扫才能按码筛选。
    ///
    /// 列语义（刻意区分两态，否则回填无法自终止）：
    /// - `NULL` = 尚未判定（待回填）
    /// - `''`   = 已判定：**该曲目无风格码**
    /// - `'-Y23-S01-'` = 有码
    ///
    /// 因此**无码的行也必须写值**（写 `''`），否则下次打开仍被当成"待回填"反复扫描。
    /// 回填完成后不再有 NULL 行 → 后续调用是「一条 SELECT 返回 0 行」的空操作。
    ///
    /// 返回回填行数。
    pub fn backfill_style_codes(&self) -> Result<usize, NcmError> {
        // 分步绑定：`query_map` 的 `MappedRows` 借用 `st`，若在同一表达式里链式
        // collect 到块尾，`st` 会先于借用被丢弃（E0597）。
        let mut st = self
            .conn
            .prepare("SELECT id, path FROM tracks WHERE style_code IS NULL")
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let mapped = st
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let pending: Vec<(i64, String)> = mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        if pending.is_empty() {
            return Ok(0);
        }
        // 单事务：中途失败整体回滚，下次打开安全重试（不留半回填状态）
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        {
            let mut up = tx
                .prepare("UPDATE tracks SET style_code = ?1 WHERE id = ?2")
                .map_err(|e| NcmError::Db(e.to_string()))?;
            for (id, path) in &pending {
                let key = crate::stylecode::style_code_key(std::path::Path::new(path))
                    .unwrap_or_default();
                up.execute(rusqlite::params![key, id])
                    .map_err(|e| NcmError::Db(e.to_string()))?;
            }
        }
        tx.commit().map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(pending.len())
    }

    /// 分页读取曲目（P6.21 排序 + P6.25 文本过滤）——**不含**风格码筛选。
    ///
    /// 委托给 [`Self::list_tracks_filtered`]（风格码传 `None`）：单一实现保证两处
    /// 谓词不会漂移。
    pub fn list_tracks_with(
        &self,
        sort: TrackSort,
        limit: i64,
        offset: i64,
        query: Option<&str>,
    ) -> Result<Vec<TrackRow>, NcmError> {
        self.list_tracks_filtered(sort, limit, offset, query, None)
    }
    /// 分页读取曲目（P6.21：支持排序；`Default` = path 稳定序，不过滤）。
    pub fn list_tracks_sorted(
        &self,
        sort: TrackSort,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TrackRow>, NcmError> {
        self.list_tracks_with(sort, limit, offset, None)
    }
    /// 分页读取曲目（按 path 稳定排序；P6.21 委托给 [`list_tracks_sorted`]）。
    pub fn list_tracks(&self, limit: i64, offset: i64) -> Result<Vec<TrackRow>, NcmError> {
        self.list_tracks_sorted(TrackSort::Default, limit, offset)
    }
    /// 艺术家聚合列表（只含 ≥1 首曲目的艺术家；脏数据自愈）。
    pub fn list_artists(&self) -> Result<Vec<ArtistRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT ar.id, ar.name, COUNT(t.id) AS n
                 FROM artists ar JOIN tracks t ON t.artist_id = ar.id
                 GROUP BY ar.id, ar.name
                 ORDER BY n DESC, ar.name",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ArtistRow {
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
    /// 专辑聚合列表（只含 ≥1 首曲目的专辑）。
    pub fn list_albums(&self) -> Result<Vec<AlbumRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT al.id, al.title, ar.name, al.year, COUNT(t.id) AS n, al.cover_cache
                 FROM albums al
                 JOIN tracks t ON t.album_id = al.id
                 LEFT JOIN artists ar ON ar.id = al.album_artist_id
                 GROUP BY al.id
                 ORDER BY n DESC, al.title",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AlbumRow {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    artist: r.get(2)?,
                    year: r.get(3)?,
                    track_count: r.get(4)?,
                    cover_path: r.get(5)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }
    /// 写入专辑封面缓存路径（文件由调用方负责落盘）。
    pub fn set_album_cover(&self, album_id: i64, cover_path: &str) -> Result<(), NcmError> {
        self.conn
            .execute(
                "UPDATE albums SET cover_cache = ?1 WHERE id = ?2",
                params![cover_path, album_id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }
    /// 回填专辑年份（在线元数据：MusicBrainz first-release-date）。
    /// **本地已有年份时不覆盖**（`year IS NULL` 守卫——用户标签优先）。
    pub fn set_album_year(&self, album_id: i64, year: i64) -> Result<(), NcmError> {
        self.conn
            .execute(
                "UPDATE albums SET year = ?1 WHERE id = ?2 AND year IS NULL",
                params![year, album_id],
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(())
    }
    /// 某艺术家的全部曲目（按专辑名 / 碟号 / 轨号排序——详情页播放序）。
    pub fn tracks_by_artist(&self, artist_id: i64) -> Result<Vec<TrackRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{TRACK_SELECT}
                 WHERE t.artist_id = ?1
                 ORDER BY al.title, t.disc_no, t.track_no, t.title"
            ))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([artist_id], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }
    /// 某专辑的曲目（按碟号 / 轨号排序——详情页播放序）。
    pub fn tracks_by_album(&self, album_id: i64) -> Result<Vec<TrackRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{TRACK_SELECT}
                 WHERE t.album_id = ?1
                 ORDER BY t.disc_no, t.track_no, t.title"
            ))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([album_id], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }
    /// 按 id 取单曲（显示名已解析；不存在 → None）。
    pub fn get_track(&self, track_id: i64) -> Result<Option<TrackRow>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(&format!("{TRACK_SELECT} WHERE t.id = ?1"))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([track_id], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows.into_iter().next())
    }
    /// 某曲目所在专辑的封面路径（底栏封面用；无专辑 / 无封面 → None）。
    pub fn track_cover_path(&self, track_id: i64) -> Result<Option<String>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT al.cover_cache
                 FROM tracks t JOIN albums al ON al.id = t.album_id
                 WHERE t.id = ?1",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([track_id], |r| r.get::<_, Option<String>>(0))
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows.into_iter().next().flatten())
    }
    /// 某艺术家曲目数最多的专辑（id + 封面路径）——艺术家图片的**降级来源**：
    /// 没有免 key 的艺术家图片源时，用其最热门专辑的封面代表（P6 决策，见记忆）。
    pub fn artist_top_album(
        &self,
        artist_id: i64,
    ) -> Result<Option<(i64, Option<String>)>, NcmError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT al.id, al.cover_cache
                 FROM tracks t JOIN albums al ON al.id = t.album_id
                 WHERE t.artist_id = ?1
                 GROUP BY al.id
                 ORDER BY COUNT(t.id) DESC
                 LIMIT 1",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([artist_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows.into_iter().next())
    }
    /// 尚无封面的专辑（id、标题、专辑艺人名）——在线封面补全的输入，
    /// 按曲目数降序（优先补最可见的专辑）。
    pub fn albums_missing_cover(
        &self,
        limit: i64,
    ) -> Result<Vec<(i64, String, Option<String>)>, NcmError> {
        let limit = limit.clamp(1, 500);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT al.id, al.title, ar.name
                 FROM albums al
                 JOIN tracks t ON t.album_id = al.id
                 LEFT JOIN artists ar ON ar.id = al.album_artist_id
                 WHERE al.cover_cache IS NULL
                 GROUP BY al.id
                 ORDER BY COUNT(t.id) DESC, al.title
                 LIMIT ?1",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }
    /// 搜索曲目（标题 / 艺术家 / 专辑 / 路径，大小写不敏感 LIKE）。
    ///
    /// 通配符转义：用户输入的 `%` `_` `\` 按字面匹配（SQL 侧 `ESCAPE '\'`）——
    /// 否则输入一个 `%` 会命中全库。
    pub fn search_tracks(&self, query: &str, limit: i64) -> Result<Vec<TrackRow>, NcmError> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let pattern = format!("%{}%", escape_like(q));
        let limit = limit.clamp(1, 500);
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{TRACK_SELECT}
                 WHERE t.title LIKE ?1 ESCAPE '\\'
                    OR ar.name LIKE ?1 ESCAPE '\\'
                    OR al.title LIKE ?1 ESCAPE '\\'
                    OR t.path LIKE ?1 ESCAPE '\\'
                 ORDER BY t.path LIMIT ?2"
            ))
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let rows = stmt
            .query_map(params![pattern, limit], map_track_row)
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;
        Ok(rows)
    }
    /// 全局搜索（P6.14）：曲目 + 专辑 + 艺术家 + 歌单，每组各 `limit` 条。
    ///
    /// 曲目谓词与 `search_tracks` 完全一致（标题/艺术家/专辑/路径；通配符按字面
    /// 转义）；空查询 → 四组全空。一次查询四组——搜索面板开一次只发一次 IPC。
    pub fn search_all(&self, query: &str, limit: i64) -> Result<SearchHits, NcmError> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(SearchHits::default());
        }
        let pattern = format!("%{}%", escape_like(q));
        let lim = limit.clamp(1, 50);

        let tracks = self.search_tracks(q, lim)?;

        // 专辑：标题或专辑艺人名命中（只含 ≥1 首曲目的专辑）
        let mut stmt = self
            .conn
            .prepare(
                "SELECT al.id, al.title, ar.name, al.year, COUNT(t.id) AS n, al.cover_cache
                 FROM albums al
                 JOIN tracks t ON t.album_id = al.id
                 LEFT JOIN artists ar ON ar.id = al.album_artist_id
                 WHERE al.title LIKE ?1 ESCAPE '\\' OR ar.name LIKE ?1 ESCAPE '\\'
                 GROUP BY al.id
                 ORDER BY n DESC, al.title
                 LIMIT ?2",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let albums = stmt
            .query_map(params![pattern, lim], |r| {
                Ok(AlbumRow {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    artist: r.get(2)?,
                    year: r.get(3)?,
                    track_count: r.get(4)?,
                    cover_path: r.get(5)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;

        // 艺术家（只含有曲目的）
        let mut stmt = self
            .conn
            .prepare(
                "SELECT ar.id, ar.name, COUNT(t.id) AS n
                 FROM artists ar JOIN tracks t ON t.artist_id = ar.id
                 WHERE ar.name LIKE ?1 ESCAPE '\\'
                 GROUP BY ar.id, ar.name
                 ORDER BY n DESC, ar.name
                 LIMIT ?2",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let artists = stmt
            .query_map(params![pattern, lim], |r| {
                Ok(ArtistRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    track_count: r.get(2)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;

        // 歌单（按名；空歌单也可见——它是用户资产）
        let mut stmt = self
            .conn
            .prepare(
                "SELECT p.id, p.name, \
                        COUNT(pi.track_id) FILTER (WHERE EXISTS (SELECT 1 FROM tracks t WHERE t.id = pi.track_id)) AS n \
                 FROM playlists p LEFT JOIN playlist_items pi ON pi.playlist_id = p.id
                 WHERE p.name LIKE ?1 ESCAPE '\\'
                 GROUP BY p.id, p.name
                 ORDER BY p.created_at DESC
                 LIMIT ?2",
            )
            .map_err(|e| NcmError::Db(e.to_string()))?;
        let playlists = stmt
            .query_map(params![pattern, lim], |r| {
                Ok(PlaylistRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    track_count: r.get(2)?,
                })
            })
            .map_err(|e| NcmError::Db(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| NcmError::Db(e.to_string()))?;

        Ok(SearchHits {
            tracks,
            albums,
            artists,
            playlists,
        })
    }
}

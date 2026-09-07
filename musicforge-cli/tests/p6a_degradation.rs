//! P6a 降级完整性验收（硬验收，handover §6.9）：**插件全禁/全挂 → 本地五域 100% 可用**。
//!
//! 五域 = 扫描 / 清洗 / 去重 / 归档 / 转换。本测试在零插件环境（默认构建不含
//! `plugin-host` feature，CLI 不链接任何插件符号——CI feature 断言钉住）跑通
//! 五域核心路径；任何一域若隐式依赖插件进程/网络，本测试即失败。
//!
//! 全部内容自构造（内嵌 NCM 编码器，与 `qa_adversarial.rs` 同源算法），无版权材料。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use musicforge_cli::{run_with_progress_expanded, BatchConfig};
use musicforge_core::scan::{self, scan_library};

// ---------- 自建 NCM 编码器（与 musicforge-cli/tests/qa_adversarial.rs 同源） ----------

mod ncm {
    use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
    use aes::Aes128;

    pub const MAGIC: &[u8; 8] = b"CTENFDAM";

    pub fn aes_ecb_encrypt(key: &[u8; 16], data: &[u8]) -> Vec<u8> {
        let cipher = Aes128::new(GenericArray::from_slice(key));
        let pad = 16 - data.len() % 16;
        let mut out = data.to_vec();
        out.extend(std::iter::repeat_n(pad as u8, pad));
        for chunk in out.as_chunks_mut::<16>().0 {
            cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
        }
        out
    }

    pub fn rc4_ksa(key: &[u8]) -> [u8; 256] {
        let mut s = [0u8; 256];
        for (i, b) in s.iter_mut().enumerate() {
            *b = i as u8;
        }
        let mut j = 0usize;
        for i in 0..256 {
            j = (j + s[i] as usize + key[i % key.len()] as usize) & 0xff;
            s.swap(i, j);
        }
        s
    }

    pub fn ncm_crypt(data: &[u8], box_: &[u8; 256]) -> Vec<u8> {
        let mut out = data.to_vec();
        for (i, b) in out.iter_mut().enumerate() {
            let j = (i + 1) & 0xff;
            *b ^= box_[(box_[j] as usize + box_[(box_[j] as usize + j) & 0xff] as usize) & 0xff];
        }
        out
    }

    pub fn build_meta_raw(music_name: &str, album: &str, fmt: &str) -> Vec<u8> {
        use base64::Engine as _;
        let meta_json = serde_json::json!({
            "musicId": 1, "musicName": music_name,
            "artist": [["测试歌手", 0]],
            "albumId": 1, "album": album,
            "albumPic": "", "bitrate": 320000, "duration": 1000,
            "format": fmt,
        })
        .to_string();
        let inner = aes_ecb_encrypt(
            &musicforge_core::META_KEY,
            format!("music:{meta_json}").as_bytes(),
        );
        let full = format!(
            "163 key(Don't modify):{}",
            base64::engine::general_purpose::STANDARD.encode(inner)
        );
        full.bytes().map(|b| b ^ 0x63).collect()
    }

    pub fn build_key_data(rc4_key: &[u8]) -> Vec<u8> {
        let plain = [b"neteasecloudmusic".as_slice(), rc4_key].concat();
        aes_ecb_encrypt(&musicforge_core::CORE_KEY, &plain)
            .iter()
            .map(|b| b ^ 0x64)
            .collect()
    }

    pub fn standard_ncm(audio: &[u8], music_name: &str, album: &str, fmt: &str) -> Vec<u8> {
        let rc4_key: Vec<u8> = (0..112).map(|i| (i * 7 + 13) as u8).collect();
        let key_data = build_key_data(&rc4_key);
        let meta_raw = build_meta_raw(music_name, album, fmt);
        let audio_enc = ncm_crypt(audio, &rc4_ksa(&rc4_key));
        let mut head = Vec::new();
        head.extend_from_slice(MAGIC);
        head.extend_from_slice(&[0x01, 0x69]);
        head.extend_from_slice(&(key_data.len() as u32).to_le_bytes());
        head.extend_from_slice(&key_data);
        head.extend_from_slice(&(meta_raw.len() as u32).to_le_bytes());
        head.extend_from_slice(&meta_raw);
        let crc = crc32fast::hash(&head);
        head.extend_from_slice(&crc.to_le_bytes());
        head.push(0x01);
        head.extend_from_slice(&0u32.to_le_bytes());
        head.extend_from_slice(&0u32.to_le_bytes());
        head.extend_from_slice(&audio_enc);
        head
    }

    pub fn minimal_flac() -> Vec<u8> {
        let mut info = Vec::new();
        info.extend_from_slice(&4096u16.to_be_bytes());
        info.extend_from_slice(&4096u16.to_be_bytes());
        info.extend_from_slice(&[0u8; 3]);
        info.extend_from_slice(&[0u8; 3]);
        let packed: u64 = (44100u64 << 44) | (1u64 << 41) | (15u64 << 36);
        info.extend_from_slice(&packed.to_be_bytes());
        info.extend_from_slice(&[0u8; 16]);
        let mut out = b"fLaC".to_vec();
        out.push(0x80);
        out.extend_from_slice(&(info.len() as u32).to_be_bytes()[1..]);
        out.extend_from_slice(&info);
        out.extend_from_slice(&[0u8; 512]);
        out
    }
}

/// 造一个微型曲库。治理域（扫描/清洗/去重/归档）面向已转库音频（.flac）；
/// 转换域输入（.ncm）放独立 `source/` 子目录。
///
/// 布局：
/// - `lib/`：dup_a.flac + dup_b.flac（同内容 → 去重组）、solo.flac（唯一）、
///   Thumbs.db（清洗素材）、solo.lrc / cover.jpg（归档吸入素材）
/// - `source/`：2 个合法 .ncm（转换域素材）
fn make_library(root: &Path) -> Vec<PathBuf> {
    let lib = root.join("lib");
    let music = lib.join("music");
    std::fs::create_dir_all(&music).unwrap();
    let flac = ncm::minimal_flac();
    let mut solo_flac = ncm::minimal_flac();
    solo_flac.push(0x00); // 内容不同 → 不与重复组混淆
    std::fs::write(music.join("dup_a.flac"), &flac).unwrap();
    std::fs::write(music.join("dup_b.flac"), &flac).unwrap();
    std::fs::write(music.join("solo.flac"), &solo_flac).unwrap();
    std::fs::write(lib.join("Thumbs.db"), b"junk").unwrap();
    std::fs::write(lib.join("solo.lrc"), "[00:00.00] 测试歌词").unwrap();
    std::fs::write(lib.join("cover.jpg"), b"fake-cover").unwrap();

    let src = root.join("source");
    std::fs::create_dir_all(&src).unwrap();
    let a = src.join("曲甲.ncm");
    let b = src.join("曲乙.ncm");
    std::fs::write(&a, ncm::standard_ncm(&flac, "曲甲", "专辑", "flac")).unwrap();
    std::fs::write(&b, ncm::standard_ncm(&flac, "曲乙", "专辑", "flac")).unwrap();
    vec![a, b]
}

/// 硬验收：插件全禁（零插件环境）→ 五域全部可用。
#[test]
fn with_plugins_disabled_all_five_local_domains_fully_usable() {
    let root = tempfile::tempdir().unwrap();
    let ncm_inputs = make_library(root.path());
    let lib = root.path().join("lib");

    // ---- 域 1：扫描 ----
    let report = scan_library(&lib, &scan::ScanOptions::default()).unwrap();
    assert_eq!(report.audio, 3, "3 个 .flac 应被识别为音频");
    assert_eq!(
        report.scanned_files, 6,
        "扫描应覆盖全部库文件（3 音频+歌词+封面+垃圾）"
    );
    // junk=3：Thumbs.db（垃圾名）+ solo.lrc / cover.jpg（音频在 music/ 子目录，
    // 二者按「孤立歌词/孤立封面」规则归入垃圾类——扫描域规则按设计工作）
    assert_eq!(
        report.junk, 3,
        "垃圾类 = Thumbs.db + 孤立 .lrc + 孤立 cover"
    );

    // ---- 域 2：清洗（规划 + 回收站执行；牺牲项可还原语义由 P3 回归钉住）----
    let enabled: HashSet<&'static str> = scan::RULE_CARDS.iter().map(|c| c.id).collect();
    let trash_root = lib.join(".musicforge/trash");
    let clean_plan = scan::build_clean_plan(&report, &enabled, &trash_root, &lib);
    // 动作构成（按规则设计）：Thumbs.db（垃圾）+ solo.lrc / cover.jpg（与音频
    // 分处两目录 → 孤立歌词/孤立封面规则命中）。断言只钉「域可用性」口径：
    // 计划非空、全部携带已注册规则 ID、执行全部进回收站且带回滚清单。
    assert!(
        !clean_plan.actions.is_empty(),
        "清洗规划不得为空（至少 Thumbs.db 应命中）"
    );
    assert!(
        clean_plan
            .actions
            .iter()
            .all(|a| scan::rule_card(a.rule_id).is_some()),
        "全部动作必须携带已注册规则 ID"
    );
    let task_id = "t-clean-smoke";
    let outcome = scan::apply_clean_plan(&clean_plan, task_id).unwrap();
    assert_eq!(
        outcome.moved,
        clean_plan.actions.len(),
        "清洗应把全部计划项移入回收站"
    );
    assert!(outcome.rollback_manifest.is_some(), "回滚清单必须随行");
    assert!(!lib.join("Thumbs.db").exists(), "垃圾项应已入回收站");

    // ---- 域 3：去重（重复组识别 + 计划构建，默认画像）----
    let dup_report = musicforge_core::dedupe::dedupe_scan(&lib, &Default::default(), None).unwrap();
    assert_eq!(
        dup_report.groups.len(),
        1,
        "两个同内容 .flac 应构成 1 组: {dup_report:?}"
    );
    let dup_plan =
        musicforge_core::dedupe::build_dedupe_plan(&dup_report, &trash_root, &lib, false);
    assert_eq!(
        dup_plan.actions.len(),
        1,
        "exact 组牺牲项 = 1（仅规划，不执行）"
    );

    // ---- 域 4：归档（organize 规划；冲突策略永不覆盖）----
    let organize_plan = musicforge_core::organize::plan_organize(
        &lib,
        &musicforge_core::organize::OrganizeOptions {
            template: "{album}/{title}",
            target_root: &lib,
            conflict: musicforge_core::organize::ConflictStrategy::Suffix,
        },
    )
    .unwrap();
    assert!(
        organize_plan.counts().planned > 0,
        "归档规划应产出待移动项（music/ 子目录 → 目标根；存量 .lrc/cover 不阻断）"
    );

    // ---- 域 5：转换（ncm → flac 全链路解密+渲染+落盘）----
    let out = tempfile::tempdir().unwrap();
    let cfg = BatchConfig {
        inputs: Vec::new(),
        out_dir: Some(out.path().to_path_buf()),
        recursive: false,
        skip_existing: false,
        jobs: 1,
        template: "{title}".to_string(),
        cancel: None,
        dry_run: false,
        manifest: None,
    };
    let pairs: Vec<(PathBuf, Option<PathBuf>)> = ncm_inputs
        .iter()
        .map(|p| (p.clone(), Some(root.path().join("source"))))
        .collect();
    let summary = run_with_progress_expanded(pairs, cfg, |_| {});
    assert_eq!(summary.failed, 0, "零插件环境转换必须零失败: {summary:?}");
    assert_eq!(summary.ok, 2, "2 个 ncm 应全部转换成功");
    assert!(out.path().join("曲甲.flac").exists(), "产物必须落盘");

    // ---- 结构性断言：以上五域全程未 spawn 任何插件进程 ----
    // （默认构建下 CLI 无 plugin-host 符号——CI `cargo tree` feature 断言 + 本测试
    //   共同构成降级完整性的验收闭环；若未来某域隐式引入插件依赖，本测试的
    //   「默认构建可编译」本身就是第一道失败信号。）
}

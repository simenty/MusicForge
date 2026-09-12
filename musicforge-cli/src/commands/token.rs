//! 服务端 token 管理（P2-3 安全加固）：fpk / 自建 server 形态的访问凭据轮换。
//!
//! 立场（为什么 CLI 而不是 HTTP 端点）：
//! - 轮换是**本机管理动作**（需要文件系统权限）——放 HTTP 面等于把凭据管理暴露给
//!   网络，与 token 闸的初衷相悖；
//! - 只动 token 文件，不触碰曲库与数据目录的其它内容。
//!
//! 生效时机：server 在启动时把 token 读进内存——**轮换后需重启服务**
//! （fpk：fnOS 应用中心「停用 → 启用」；裸跑：SIGTERM 后重新拉起）。
//!
//! 规格一致性：生成的 token 与 server 端同规格（24B 随机 → 48 位十六进制，
//! 文件权限 0o600）。若修改生成逻辑，请同步 `musicforge-server/src/lib.rs`
//! 的 `generate_token` / `load_or_create_token`。

use std::path::PathBuf;

// 与同为「非迁移块」的 plugins.rs 一致：从 crate root（bin）取 Sub/TokenAction 等类型。
use crate::*;

/// 解析 token 文件路径（优先级从高到低）：
/// `--token-file` > `--data-dir/.token` > `MUSICFORGE_TOKEN_FILE`
/// > `MUSICFORGE_DATA_DIR/.token` > `./data/.token`
pub fn resolve_token_file(token_file: Option<&str>, data_dir: Option<&str>) -> PathBuf {
    if let Some(f) = token_file {
        return PathBuf::from(f);
    }
    if let Some(d) = data_dir {
        return PathBuf::from(d).join(".token");
    }
    if let Ok(f) = std::env::var("MUSICFORGE_TOKEN_FILE") {
        let f = f.trim();
        if !f.is_empty() {
            return PathBuf::from(f);
        }
    }
    let d = std::env::var("MUSICFORGE_DATA_DIR").unwrap_or_else(|_| "data".to_string());
    PathBuf::from(d).join(".token")
}

/// 24B 随机 → 48 位十六进制（与 server 端同规格）。
fn generate_token() -> String {
    #[cfg(unix)]
    {
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let mut buf = [0u8; 24];
            if f.read_exact(&mut buf).is_ok() {
                return buf.iter().map(|b| format!("{b:02x}")).collect();
            }
        }
    }
    // 兜底（非 unix / urandom 不可用）：RandomState 多轮混淆 + 时间/pid
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut acc: u128 = std::process::id() as u128;
    for i in 0..6u128 {
        let h = std::collections::hash_map::RandomState::new();
        let mut hasher = h.build_hasher();
        hasher.write_u128(acc ^ (i << 96));
        acc = acc.rotate_left(29)
            ^ (hasher.finish() as u128)
            ^ (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as u128)
                .unwrap_or(0)
                << (i * 13 % 96));
    }
    format!("{acc:032x}{:016x}", acc as u64 ^ (acc >> 64) as u64)
}

/// 写入 token 文件（unix 下 0o600——凭据不得被同机其他用户读取）。
fn write_token_file(path: &std::path::Path, token: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| format!("目录创建失败: {e}"))?;
        }
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("token 写入失败: {e}"))?;
        f.write_all(token.as_bytes())
            .map_err(|e| format!("token 写入失败: {e}"))?;
    }
    #[cfg(not(unix))]
    std::fs::write(path, token).map_err(|e| format!("token 写入失败: {e}"))?;
    Ok(())
}

/// `musicforge token rotate|show` 分派。
pub fn run_token_sub(a: &TokenAction) -> i32 {
    match a {
        TokenAction::Show {
            token_file,
            data_dir,
        } => {
            let path = resolve_token_file(token_file.as_deref(), data_dir.as_deref());
            match std::fs::read_to_string(&path) {
                Ok(t) => {
                    println!("token 文件: {}", path.display());
                    println!("{}", t.trim());
                    0
                }
                Err(e) => {
                    eprintln!("X 读取失败 {}: {e}", path.display());
                    eprintln!(
                        "  提示：--token-file / --data-dir 显式指定，或用 MUSICFORGE_DATA_DIR"
                    );
                    1
                }
            }
        }
        TokenAction::Rotate {
            token_file,
            data_dir,
        } => {
            let path = resolve_token_file(token_file.as_deref(), data_dir.as_deref());
            let token = generate_token();
            if let Err(e) = write_token_file(&path, &token) {
                eprintln!("X {e}");
                return 1;
            }
            println!("已轮换 token：{}", path.display());
            println!("{token}");
            println!("⚠ 需重启服务后生效（fnOS：应用中心「停用 → 启用」；裸跑：重启进程）");
            println!("  客户端请同步更新 SPA 标题栏的 token 输入框");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_format_is_48_hex_and_random() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 48, "24B hex 表示恒 48 字符: {a}");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "两次生成必须不同");
    }

    #[test]
    fn resolve_prefers_explicit_args() {
        let p = resolve_token_file(Some("/tmp/x/.token"), Some("/tmp/y"));
        assert_eq!(p, PathBuf::from("/tmp/x/.token"));
        let p = resolve_token_file(None, Some("/tmp/y"));
        assert_eq!(p, PathBuf::from("/tmp/y/.token"));
    }

    #[test]
    fn rotate_writes_file_and_reads_back() {
        let dir = std::env::temp_dir().join(format!(
            "mf-token-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join(".token");
        let token = generate_token();
        write_token_file(&path, &token).unwrap();
        let back = std::fs::read_to_string(&path).unwrap();
        assert_eq!(back, token);
        // 覆写（轮换语义）
        let t2 = generate_token();
        write_token_file(&path, &t2).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), t2);
        assert_ne!(t2, token);
        std::fs::remove_dir_all(&dir).ok();
    }
}

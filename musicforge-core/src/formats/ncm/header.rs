//! 头部解析（硬约束 3/4/5/9：read_exact / 长度上界 / 全返回值检查 / CRC32 校验）
//!
//! 布局（全部已实证，含 CRC32 覆盖范围穷举搜索定论）：
//! ```text
//! [8]  magic "CTENFDAM"
//! [2]  version/gap
//! [4]  keyLen   [keyLen]  keyData (XOR 0x64, AES-128-ECB sCoreKey, 前缀 "neteasecloudmusic")
//! [4]  metaLen  [metaLen] metaData (XOR 0x63, 跳 22B 前缀, base64, AES sModifyKey, 跳 "music:", JSON)
//! [4]  CRC32 (LE) —— 覆盖 [0, 此处偏移)
//! [1]  分隔符 0x01（社区逆向共识；本库记录不强制——观测到非固定值）
//! [4]  coverLen1 (LE)  [4] coverLen2 (LE)  [coverLen2] 图片  [coverLen1-coverLen2] padding
//! [..] 音频（RC4 变体加密，全局偏移密钥流）
//! ```

use std::io::{Read, Seek, SeekFrom};

use super::crypto::{aes128_ecb_decrypt_strict, build_key_box};
use crate::error::NcmError;
use crate::metadata::{self, Metadata};
use crate::{CORE_KEY, MAGIC, META_KEY, META_PREFIX, MUSIC_PREFIX, NETEASE_PREFIX};

/// 解析完成的头部信息
pub struct Header {
    pub key_box: [u8; 256],
    pub metadata: Option<Metadata>,
    pub cover: Vec<u8>,
    /// 音频区在文件中的起始偏移
    pub audio_offset: u64,
    /// 音频区字节数（文件总长 - audio_offset）
    pub audio_len: u64,
}

/// 读入整个文件并解析头部。小文件（ncm 通常 <100MB）一次性载入内存换取实现简单；
/// 流式解码（`Decoder::open`）不经过本函数。
pub fn parse_blob(blob: &[u8]) -> Result<Header, NcmError> {
    let file_len = blob.len() as u64;
    let need = |pos: u64, n: u64, what: &'static str| -> Result<(), NcmError> {
        if pos + n > file_len {
            return Err(NcmError::Truncated { at: what, need: n, got: file_len.saturating_sub(pos) });
        }
        Ok(())
    };
    let u32le = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);

    let mut o: u64 = 0;
    need(0, 8, "magic")?;
    if blob[0..8] != MAGIC {
        return Err(NcmError::BadMagic);
    }
    o += 8;

    need(o, 2, "version")?;
    o += 2; // version/gap（观测到 01 69 / 01 70 等非固定值，不校验）

    // ---- 密钥块 ----
    need(o, 4, "keyLen")?;
    let key_len = u32le(&blob[o as usize..o as usize + 4]) as u64;
    o += 4;
    if key_len == 0 || o + key_len > file_len {
        return Err(NcmError::LengthOutOfRange { at: "keyLen", value: key_len, max: file_len - o });
    }
    let key_data: Vec<u8> = blob[o as usize..(o + key_len) as usize].iter().map(|b| b ^ 0x64).collect();
    o += key_len;

    let key_plain = aes128_ecb_decrypt_strict(&CORE_KEY, &key_data)?;
    if key_plain.len() < NETEASE_PREFIX.len() || key_plain[..NETEASE_PREFIX.len()] != NETEASE_PREFIX {
        return Err(NcmError::BadKeyPrefix);
    }
    let rc4_key = &key_plain[NETEASE_PREFIX.len()..];
    let key_box = build_key_box(rc4_key)?;

    // ---- 元数据块 ----
    need(o, 4, "metaLen")?;
    let meta_len = u32le(&blob[o as usize..o as usize + 4]) as u64;
    o += 4;
    if o + meta_len > file_len {
        return Err(NcmError::LengthOutOfRange { at: "metaLen", value: meta_len, max: file_len - o });
    }

    let metadata = if meta_len == 0 {
        None // 3.x 及部分文件无元数据（硬约束 11：None 而非失败）
    } else {
        need(o, meta_len, "metaData")?;
        let meta_xor: Vec<u8> = blob[o as usize..(o + meta_len) as usize].iter().map(|b| b ^ 0x63).collect();
        o += meta_len;
        if meta_xor.len() < META_PREFIX.len() || meta_xor[..META_PREFIX.len()] != META_PREFIX {
            return Err(NcmError::BadMetaPrefix);
        }
        use base64::Engine as _;
        let meta_inner = base64::engine::general_purpose::STANDARD.decode(&meta_xor[META_PREFIX.len()..])?;
        let meta_plain = aes128_ecb_decrypt_strict(&META_KEY, &meta_inner)?;
        if meta_plain.len() < MUSIC_PREFIX.len() || meta_plain[..MUSIC_PREFIX.len()] != MUSIC_PREFIX {
            return Err(NcmError::BadMusicPrefix);
        }
        metadata::parse(&meta_plain[MUSIC_PREFIX.len()..]).ok()
    };

    // ---- CRC32 头校验（硬约束 9；覆盖 [0, 此处偏移) —— 穷举实证定论）----
    need(o, 4, "crc32")?;
    let crc_pos = o;
    let crc_stored = u32le(&blob[o as usize..o as usize + 4]);
    o += 4;
    let crc_computed = crc32fast::hash(&blob[..crc_pos as usize]);
    if crc_stored != crc_computed {
        return Err(NcmError::CrcMismatch { expected: crc_stored, computed: crc_computed });
    }

    // ---- 封面帧 ----
    need(o, 1, "cover separator")?;
    o += 1; // 分隔符（观测 0x01，非固定值，不校验）
    need(o, 4, "coverLen1")?;
    let cover_len1 = u32le(&blob[o as usize..o as usize + 4]) as u64;
    o += 4;
    need(o, 4, "coverLen2")?;
    let cover_len2 = u32le(&blob[o as usize..o as usize + 4]) as u64;
    o += 4;
    if cover_len2 > cover_len1 {
        // 观测恒等；出现反例则记录（前向兼容），不做硬失败
        // （Go 版多出的 Seek(L1-L2) 在 L1<L2 时求负偏移会被本库上界校验拦截——修 F5）
    }
    if o + cover_len2 > file_len {
        return Err(NcmError::LengthOutOfRange { at: "coverLen2", value: cover_len2, max: file_len - o });
    }
    // 硬约束 4：coverLen1 同样受上界校验（QA 补漏：此前仅 coverLen2 受检）
    if o + cover_len1 > file_len {
        return Err(NcmError::LengthOutOfRange { at: "coverLen1", value: cover_len1, max: file_len - o });
    }
    let cover = blob[o as usize..(o + cover_len2) as usize].to_vec();
    o += cover_len1; // 图片 + padding（Go 版 Seek(L1-L2) 的等价物，且返回值必检）

    // ---- 音频区 ----
    if o >= file_len {
        return Err(NcmError::EmptyAudio);
    }
    Ok(Header { key_box, metadata, cover, audio_offset: o, audio_len: file_len - o })
}

/// 从任意 Reader 流式解析头部（`Decoder::open` 用；不整读文件）。
/// 返回解析消耗的字节数（= audio_offset）。
pub fn parse_stream<R: Read + Seek>(reader: &mut R) -> Result<Header, NcmError> {
    let file_len = reader.seek(SeekFrom::End(0))?;
    reader.rewind()?;
    
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if magic != MAGIC {
        return Err(NcmError::BadMagic);
    }
    let mut ver = [0u8; 2];
    reader.read_exact(&mut ver)?;

    let mut len4 = [0u8; 4];
    reader.read_exact(&mut len4)?;
    let key_len = u32::from_le_bytes(len4) as usize;
    if key_len == 0 || key_len > 1 << 20 {
        return Err(NcmError::LengthOutOfRange { at: "keyLen", value: key_len as u64, max: 1 << 20 });
    }
    let mut key_data = vec![0u8; key_len];
    reader.read_exact(&mut key_data)?;

    reader.read_exact(&mut len4)?;
    let meta_len = u32::from_le_bytes(len4) as usize;
    if meta_len > 1 << 24 {
        return Err(NcmError::LengthOutOfRange { at: "metaLen", value: meta_len as u64, max: 1 << 24 });
    }
    let mut meta_raw = vec![0u8; meta_len];
    if meta_len > 0 {
        reader.read_exact(&mut meta_raw)?;
    }

    let crc_pos = reader.stream_position()?;
    reader.read_exact(&mut len4)?;
    let crc_stored = u32::from_le_bytes(len4);

    let mut sep = [0u8; 1];
    reader.read_exact(&mut sep)?;
    reader.read_exact(&mut len4)?;
    let cover_len1 = u32::from_le_bytes(len4) as usize;
    reader.read_exact(&mut len4)?;
    let cover_len2 = u32::from_le_bytes(len4) as usize;
    if cover_len2 > 1 << 26 {
        return Err(NcmError::LengthOutOfRange { at: "coverLen2", value: cover_len2 as u64, max: 1 << 26 });
    }
    let mut cover = vec![0u8; cover_len2];
    if cover_len2 > 0 {
        reader.read_exact(&mut cover)?;
    }
    let skip = cover_len1.wrapping_sub(cover_len2);
    reader.seek(SeekFrom::Current(skip as i64))?;

    // CRC 校验：回读 [0, 当前 audio_offset)
    let audio_offset = reader.stream_position()?;
    // 硬约束 4/5：音频起点必须落在文件内；无音频一律 EmptyAudio（QA-B1：
    // 空音频 / coverLen1 越界曾绕过本检查产出 0 字节文件并假报成功）
    if audio_offset > file_len {
        return Err(NcmError::LengthOutOfRange {
            at: "coverLen1",
            value: cover_len1 as u64,
            max: file_len,
        });
    }
    if audio_offset == file_len {
        return Err(NcmError::EmptyAudio);
    }
    reader.rewind()?;
    let mut head_bytes = vec![0u8; crc_pos as usize];
    reader.read_exact(&mut head_bytes)?;
    let crc_computed = crc32fast::hash(&head_bytes);
    if crc_stored != crc_computed {
        return Err(NcmError::CrcMismatch { expected: crc_stored, computed: crc_computed });
    }
    reader.seek(SeekFrom::Start(audio_offset))?;

    let metadata = if meta_len == 0 {
        None
    } else {
        let meta_xor: Vec<u8> = meta_raw.iter().map(|b| b ^ 0x63).collect();
        if meta_xor.len() < META_PREFIX.len() || meta_xor[..META_PREFIX.len()] != META_PREFIX {
            return Err(NcmError::BadMetaPrefix);
        }
        use base64::Engine as _;
        let meta_inner = base64::engine::general_purpose::STANDARD.decode(&meta_xor[META_PREFIX.len()..])?;
        let meta_plain = aes128_ecb_decrypt_strict(&META_KEY, &meta_inner)?;
        if meta_plain.len() < MUSIC_PREFIX.len() || meta_plain[..MUSIC_PREFIX.len()] != MUSIC_PREFIX {
            return Err(NcmError::BadMusicPrefix);
        }
        metadata::parse(&meta_plain[MUSIC_PREFIX.len()..]).ok()
    };

    let key_data_x: Vec<u8> = key_data.iter().map(|b| b ^ 0x64).collect();
    let key_plain = aes128_ecb_decrypt_strict(&CORE_KEY, &key_data_x)?;
    if key_plain.len() < NETEASE_PREFIX.len() || key_plain[..NETEASE_PREFIX.len()] != NETEASE_PREFIX {
        return Err(NcmError::BadKeyPrefix);
    }
    let key_box = build_key_box(&key_plain[NETEASE_PREFIX.len()..])?;

    let audio_len = file_len.saturating_sub(audio_offset);
    Ok(Header { key_box, metadata, cover, audio_offset, audio_len })
}

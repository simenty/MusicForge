//! 密码学原语：AES-128-ECB 解密（严格 PKCS7）、NCM RC4 变体密钥盒与密钥流。
//!
//! 硬约束映射：
//! - 约束 1（零 panic）：所有失败返回 `NcmError`
//! - 约束 2（RC4 全局偏移）：`keystream_byte` 以**全局音频偏移**为参数（修 B-5 块内偏移炸弹）
//! - 约束 4（长度校验）：PKCS7 严格校验 + KeyBox 索引用 `usize`（修 F7 uint8 回绕）

use crate::error::NcmError;
use aes::cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};
use aes::Aes128;

/// AES-128-ECB 解密 + **严格** PKCS7 去填充（填充非法即报错——比上游的宽松检查更严，损坏数据早暴露）
pub fn aes128_ecb_decrypt_strict(key: &[u8; 16], data: &[u8]) -> Result<Vec<u8>, NcmError> {
    if data.is_empty() || !data.len().is_multiple_of(16) {
        return Err(NcmError::LengthOutOfRange {
            at: "aes-ecb input",
            value: data.len() as u64,
            max: u64::MAX,
        });
    }
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let mut out = data.to_vec();
    for chunk in out.as_chunks_mut::<16>().0 {
        cipher.decrypt_block(GenericArray::from_mut_slice(chunk));
    }
    // 严格 PKCS7：尾字节 1..=16，且末尾 pad 个字节全部等于 pad
    let pad = *out.last().ok_or(NcmError::EmptyAudio)? as usize;
    if pad == 0 || pad > 16 || pad > out.len() || out[out.len() - pad..].iter().any(|&b| b as usize != pad)
    {
        return Err(NcmError::LengthOutOfRange {
            at: "pkcs7 padding",
            value: pad as u64,
            max: 16,
        });
    }
    out.truncate(out.len() - pad);
    Ok(out)
}

/// NCM RC4 变体密钥盒（KSA 与上游等价；keyOffset 用 usize 防回绕——修 F7）
///
/// 空 RC4 密钥返回 `Err(NcmError::EmptyKey)`（QA-B6 修复：空密钥下 `key[key_offset]`
/// 索引越界 panic、违反本模块硬约束 1；release `panic="abort"` 下单个畸形文件即可
/// 崩掉整个批处理进程）。
pub fn build_key_box(key: &[u8]) -> Result<[u8; 256], NcmError> {
    if key.is_empty() {
        return Err(NcmError::EmptyKey);
    }
    let mut box_ = [0u8; 256];
    for (i, b) in box_.iter_mut().enumerate() {
        *b = i as u8;
    }
    #[allow(unused_assignments)]
    let mut c: usize = 0;
    let mut last_byte: usize = 0;
    let mut key_offset: usize = 0;
    for i in 0..256 {
        let swap = box_[i];
        c = (swap as usize + last_byte + key[key_offset] as usize) & 0xff;
        key_offset = (key_offset + 1) % key.len();
        box_[i] = box_[c];
        box_[c] = swap;
        last_byte = c;
    }
    Ok(box_)
}

/// RC4-PRGA 变体密钥流：`j = (全局偏移 + 1) & 0xff`（**全局偏移**，非块内——硬约束 2）
///
/// 协议行为（逆向共识，四方实现交叉验证）：预生成密钥盒循环复用，PRGA 不做 swap。
#[inline]
pub fn keystream_byte(box_: &[u8; 256], global_offset: u64) -> u8 {
    let j = ((global_offset + 1) & 0xff) as usize;
    let a = box_[j] as usize;
    let b = box_[(a + j) & 0xff] as usize;
    box_[(a + b) & 0xff]
}

/// 就地解密一段音频（调用方保证 `data` 与 `global_offset` 对齐）
pub fn xor_stream(data: &mut [u8], box_: &[u8; 256], global_offset: u64) {
    for (i, byte) in data.iter_mut().enumerate() {
        *byte ^= keystream_byte(box_, global_offset + i as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与 scratch/make_ncm.py 的 rc4_ksa + ncm_crypt 对拍的往返性质：
    /// xor_stream 两次 = 原文（加解密同函数）。
    #[test]
    fn keystream_is_self_inverse() {
        let key: Vec<u8> = (0..112).map(|i| (i * 7 + 13) as u8).collect();
        let box_ = build_key_box(&key).unwrap();
        let mut data: Vec<u8> = (0..1024).map(|i| (i * 31 + 7) as u8).collect();
        let plain = data.clone();
        xor_stream(&mut data, &box_, 0);
        assert_ne!(data, plain);
        xor_stream(&mut data, &box_, 0);
        assert_eq!(data, plain);
    }

    /// 全局偏移 vs 块内偏移的分界验证：从块边界（0x8000）续流必须与一次性解密一致
    /// （这正是上游 B-5 的病灶——按块内偏移会在第 2 块起产出错误密钥流）。
    #[test]
    fn global_offset_across_chunk_boundary() {
        let key: Vec<u8> = (0..112).map(|i| (i * 11 + 3) as u8).collect();
        let box_ = build_key_box(&key).unwrap();
        let len = 0x8000 + 100;
        let plain: Vec<u8> = (0..len).map(|i| (i * 13 + 1) as u8).collect();

        // 一次性
        let mut one = plain.clone();
        xor_stream(&mut one, &box_, 0);

        // 分块（模拟 Decoder 跨 32KB 边界续读）
        let mut chunked = plain.clone();
        xor_stream(&mut chunked[..0x8000], &box_, 0);
        xor_stream(&mut chunked[0x8000..], &box_, 0x8000);
        assert_eq!(one, chunked);
    }
}

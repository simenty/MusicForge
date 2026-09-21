//! 真机音频平台诊断（P6.29）：**不在 CI 自动执行**，需人工显式运行。
//!
//! ```text
//! cargo test -p musicforge-gui --test audio_probe -- --ignored --nocapture
//! ```
//!
//! 用途：判定**当前这台机器**是否具备承载 DoP（DSD over PCM）的条件。
//! DoP 的硬性门槛：
//! - DSD64 需要 **176400 Hz** 输出流（DSD128 → 352800 Hz）；
//! - 且必须是**独占/直通**路径——系统重采样会把 DSD 载荷比特彻底打乱。
//!
//! 2026-09-22 本机实测（Windows / WASAPI / Realtek）：
//! - `default_output_config` = 2ch / 48000 Hz / F32；
//! - `supported_output_configs` 四种格式（U8/I16/I32/F32）**全部 min=max=48000**；
//! - 即 cpal 的 WASAPI 后端只暴露**共享模式混音格式**，不提供独占模式，
//!   最高 48 kHz → **DoP 在本机不可实现**。
//!   要走通需换后端（WASAPI exclusive / ASIO / KS）且硬件接受 176.4 kHz。

#[test]
#[ignore = "真机诊断：需人工显式运行（-- --ignored --nocapture），CI 不执行"]
fn probe_audio_platform() {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();
    println!("=== HOST: {:?} ===", host.id());

    let Some(dev) = host.default_output_device() else {
        println!("!! 无默认输出设备（无声卡 / 无音频端点）");
        return;
    };
    println!(
        "=== DEFAULT OUTPUT DEVICE: {} ===",
        dev.name().unwrap_or_else(|e| format!("<err {e}>"))
    );

    match dev.default_output_config() {
        Ok(c) => println!(
            "default_cfg: channels={} rate={} format={:?}",
            c.channels(),
            c.sample_rate().0,
            c.sample_format()
        ),
        Err(e) => println!("default_cfg err: {e}"),
    }

    println!("--- supported_output_configs ---");
    let mut max_rate = 0u32;
    match dev.supported_output_configs() {
        Ok(ranges) => {
            for r in ranges {
                println!(
                    "  ch={} min={} max={} buf={:?} fmt={:?}",
                    r.channels(),
                    r.min_sample_rate().0,
                    r.max_sample_rate().0,
                    r.buffer_size(),
                    r.sample_format()
                );
                max_rate = max_rate.max(r.max_sample_rate().0);
            }
        }
        Err(e) => println!("  supported_output_configs err: {e}"),
    }

    println!("--- DoP 判定 ---");
    println!("最高支持采样率 = {max_rate} Hz");
    for need in [176_400u32, 352_800] {
        println!(
            "  {need} Hz（{}）: {}",
            if need == 176_400 { "DSD64" } else { "DSD128" },
            if max_rate >= need {
                "覆盖"
            } else {
                "不支持（低于所需）"
            }
        );
    }
}

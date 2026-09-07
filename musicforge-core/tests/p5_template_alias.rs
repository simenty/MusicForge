//! P5b.2（D25）：beets / Music Tag Web 模板兼容别名。

use musicforge_core::metadata::Metadata;
use musicforge_core::template::engine::{normalize_aliases, render_filename};

fn meta() -> Metadata {
    Metadata {
        name: Some("借墨".to_string()),
        artist: Some("王铮亮".to_string()),
        album: Some("借墨".to_string()),
        format: Some("flac".to_string()),
        track: Some(1),
        bitrate: None,
        duration: Some(252000),
        album_pic_url: None,
    }
}

#[test]
fn beets_dollar_aliases_map_to_native() {
    assert_eq!(normalize_aliases("$artist/$album"), "{artist}/{album}");
    assert_eq!(normalize_aliases("$title"), "{title}");
    assert_eq!(normalize_aliases("$track"), "{track}");
}

#[test]
fn percent_aliases_map_to_native() {
    assert_eq!(
        normalize_aliases("%artist% - %title%"),
        "{artist} - {title}"
    );
}

#[test]
fn dollar_prefix_does_not_clobber_longer_names() {
    // $artists 不是 $artist（后随字母 s）→ 原样保留
    assert_eq!(normalize_aliases("$artists"), "$artists");
    // $artist+数字/结尾 → 命中
    assert_eq!(normalize_aliases("$artist2"), "{artist}2");
}

#[test]
fn unknown_dollar_names_are_untouched() {
    assert_eq!(normalize_aliases("$foo/$album"), "$foo/{album}");
}

#[test]
fn alias_and_native_render_identically() {
    let m = meta();
    let native = render_filename("{artist} - {title}", Some(&m), "fb");
    let beets = render_filename("$artist - $title", Some(&m), "fb");
    let mtw = render_filename("%artist% - %title%", Some(&m), "fb");
    assert_eq!(native, beets, "beets 别名渲染与原生一致");
    assert_eq!(native, mtw, "Music Tag Web 别名渲染与原生一致");
    assert!(native.contains("王铮亮"), "{native}");
    assert!(native.contains("借墨"), "{native}");
}

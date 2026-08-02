//! 运行时可自定义素材：exe 同目录下如果有 assets/ 文件夹，就用里面的图片
//! 替换内置默认贴图；同时把托盘图标也换成同一张"身份"图片。
//! 只在启动时判定一次(不监听文件变化)，跟核心状态机(physics.rs)完全独立，
//! 只是给前端提供一个 get_skin_config 命令、以及在 setup 阶段顺手换一下托盘图标。
//!
//! `assets_root()` / `load_external_as_data_url()` 是特意公开出来的通用零件：
//! 以后如果某个功能(比如系统状态弹窗)也想支持自定义素材，可以在 assets/
//! 下开一个自己的子文件夹(比如 assets/stats/)，直接复用这两个函数去读取、
//! 编码图片，缺文件时自己决定 fallback(通常是退回这里解析出的核心贴图)。
//! 不需要改这个文件，也不需要什么注册表/trait——核心贴图和某个功能自己的
//! 贴图是两件独立的事，各自的 Tauri command 分别返回给各自的前端页面即可。

use std::io::Cursor;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use image::ImageFormat;
use serde::Serialize;

const MAX_CUSTOM_IMAGE_DIM: u32 = 512; // 自定义素材解码后的最大边长(px)，超过的会等比缩小到这个范围内
const TRAY_ICON_SIZE: u32 = 32; // 托盘图标边长(px)

// 皮肤(素材)配置：5 张贴图各自最终使用哪个图片源(可能是内置默认，也可能是外部自定义图片
// 转成的 data URL)。前端拿到这个之后直接替换掉自己的贴图来源表，不需要知道背后的判定逻辑。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkinConfig {
    idle_src: String,
    walk1_src: String,
    walk2_src: String,
    walk3_src: String,
    drag_src: String,
}

fn bundled_src(name: &str) -> String {
    format!("assets/{name}.png")
}

/// exe 同目录下 assets/ 文件夹的完整路径(不保证存在)。
pub fn assets_root() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("assets")))
}

/// 读取一张外部图片文件，如果边长超过 MAX_CUSTOM_IMAGE_DIM 就等比缩小，
/// 最终编码成一个可以直接放进 <img src> 里的 data URL。失败(文件损坏/不是有效 PNG 等)返回 None。
pub fn load_external_as_data_url(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let img = image::load_from_memory_with_format(&bytes, ImageFormat::Png).ok()?;

    let (w, h) = (img.width(), img.height());
    let longest = w.max(h);
    let out_bytes = if longest > MAX_CUSTOM_IMAGE_DIM {
        let scale = MAX_CUSTOM_IMAGE_DIM as f32 / longest as f32;
        let new_w = ((w as f32) * scale).round().max(1.0) as u32;
        let new_h = ((h as f32) * scale).round().max(1.0) as u32;
        let resized = img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3);
        let mut buf = Cursor::new(Vec::new());
        resized.write_to(&mut buf, ImageFormat::Png).ok()?;
        buf.into_inner()
    } else {
        bytes
    };

    Some(format!("data:image/png;base64,{}", BASE64.encode(out_bytes)))
}

/// 决定最终 5 张贴图分别用什么：
/// - exe 同目录下的 assets/ 文件夹里一张自定义图都没有 -> 全部用内置默认
/// - 有 idle 就用 idle 做 idle 状态；没有就用编号最小的 walk 顶上
/// - 3 张 walk 都齐 -> 正常循环播放这 3 张
/// - 有 1-2 张 walk -> 走路状态固定显示编号最小的那张(不循环)
/// - 一张 walk 都没有 -> 走路状态也用 idle
/// - 有自定义的 drag.png 就用它做拖拽时的贴图；没有就直接沿用 idle 那张顶上
/// 任何一步读取/解码失败，都单独回退到对应的内置默认图，不会导致整体失败。
fn resolve_skin_config() -> SkinConfig {
    let external_dir = assets_root();

    let ext_idle = external_dir
        .as_ref()
        .map(|d| d.join("idle.png"))
        .filter(|p| p.is_file());

    let ext_drag = external_dir
        .as_ref()
        .map(|d| d.join("drag.png"))
        .filter(|p| p.is_file());

    let ext_walks: Vec<PathBuf> = external_dir
        .as_ref()
        .map(|d| {
            [1, 2, 3]
                .into_iter()
                .map(|n| d.join(format!("walk{n}.png")))
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default();

    if ext_idle.is_none() && ext_drag.is_none() && ext_walks.is_empty() {
        // 完全没有自定义素材，全部使用内置默认图
        return SkinConfig {
            idle_src: bundled_src("idle"),
            walk1_src: bundled_src("walk1"),
            walk2_src: bundled_src("walk2"),
            walk3_src: bundled_src("walk3"),
            drag_src: bundled_src("drag"),
        };
    }

    // 编号最小的可用 walk 图(如果有的话)，缺 idle 时用来顶替 idle
    let smallest_walk_src = ext_walks
        .first()
        .and_then(|p| load_external_as_data_url(p));

    let idle_src = match &ext_idle {
        Some(p) => load_external_as_data_url(p).unwrap_or_else(|| bundled_src("idle")),
        None => smallest_walk_src
            .clone()
            .unwrap_or_else(|| bundled_src("idle")),
    };

    let (walk1_src, walk2_src, walk3_src) = if ext_walks.len() == 3 {
        // 3 张齐全，正常循环，各用各的
        (
            load_external_as_data_url(&ext_walks[0]).unwrap_or_else(|| bundled_src("walk1")),
            load_external_as_data_url(&ext_walks[1]).unwrap_or_else(|| bundled_src("walk2")),
            load_external_as_data_url(&ext_walks[2]).unwrap_or_else(|| bundled_src("walk3")),
        )
    } else if !ext_walks.is_empty() {
        // 只有 1-2 张，走路状态固定用编号最小的那张，不循环
        let single = smallest_walk_src.unwrap_or_else(|| bundled_src("walk1"));
        (single.clone(), single.clone(), single)
    } else {
        // 一张 walk 都没有，走路状态也用 idle 顶上
        (idle_src.clone(), idle_src.clone(), idle_src.clone())
    };

    // 没有专门的自定义 drag 图时，直接沿用 idle 的贴图(不管 idle 本身是自定义还是内置默认)
    let drag_src = match &ext_drag {
        Some(p) => load_external_as_data_url(p).unwrap_or_else(|| idle_src.clone()),
        None => idle_src.clone(),
    };

    SkinConfig {
        idle_src,
        walk1_src,
        walk2_src,
        walk3_src,
        drag_src,
    }
}

#[tauri::command]
pub fn get_skin_config() -> SkinConfig {
    resolve_skin_config()
}

/// 决定用哪张外部图片代表宠物"身份"来当托盘图标：优先外部 idle.png，
/// 没有就用编号最小的外部 walk 图。完全没有自定义素材时返回 None——
/// 这种情况下托盘图标维持编译时生成的内置默认图标，不需要在运行时重新处理。
fn resolve_identity_image_path() -> Option<PathBuf> {
    let external_dir = assets_root()?;

    let ext_idle = external_dir.join("idle.png");
    if ext_idle.is_file() {
        return Some(ext_idle);
    }

    [1, 2, 3]
        .into_iter()
        .map(|n| external_dir.join(format!("walk{n}.png")))
        .find(|p| p.is_file())
}

/// 把一张任意尺寸/比例的图片，缩放(必要时放大)后居中贴到一张
/// TRAY_ICON_SIZE 见方的透明画布上，做成托盘图标，避免非正方形素材被直接拉伸变形。
fn build_tray_icon_from_path(path: &Path) -> Option<tauri::Icon> {
    let bytes = std::fs::read(path).ok()?;
    let img = image::load_from_memory_with_format(&bytes, ImageFormat::Png).ok()?;

    let longest = img.width().max(img.height()).max(1);
    let scale = TRAY_ICON_SIZE as f32 / longest as f32;
    let new_w = ((img.width() as f32) * scale).round().clamp(1.0, TRAY_ICON_SIZE as f32) as u32;
    let new_h = ((img.height() as f32) * scale).round().clamp(1.0, TRAY_ICON_SIZE as f32) as u32;
    let resized = img
        .resize(new_w, new_h, image::imageops::FilterType::Lanczos3)
        .to_rgba8();

    let mut canvas = image::RgbaImage::new(TRAY_ICON_SIZE, TRAY_ICON_SIZE);
    let off_x = ((TRAY_ICON_SIZE - new_w) / 2) as i64;
    let off_y = ((TRAY_ICON_SIZE - new_h) / 2) as i64;
    image::imageops::overlay(&mut canvas, &resized, off_x, off_y);

    Some(tauri::Icon::Rgba {
        rgba: canvas.into_raw(),
        width: TRAY_ICON_SIZE,
        height: TRAY_ICON_SIZE,
    })
}

/// 如果用户在 exe 同目录放了自定义素材，把托盘图标也换成对应的"身份"图片；
/// 完全没有自定义素材时不做任何事，保留编译时生成的内置默认图标。
pub fn sync_tray_icon(app: &tauri::AppHandle) {
    if let Some(identity_path) = resolve_identity_image_path() {
        if let Some(icon) = build_tray_icon_from_path(&identity_path) {
            let _ = app.tray_handle().set_icon(icon);
        }
    }
}

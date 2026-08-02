//! 多开实例之间"谁停在哪"的轻量共享登记表，用来让静止的宠物互相避开、
//! 不会叠在同一个点上。跟移动过程完全无关——只在"要不要选这个落脚点"的
//! 那一刻被查询，走路/溜达途中互相穿过、擦肩而过都不受影响。
//!
//! 实现方式很朴素：每个实例把自己的静止位置写进一个共享的临时文件(用进程 ID
//! 当 key)，读取时排除自己、排除太久没更新的过期记录(视为已经退出的实例)。
//! 数据量小、更新不频繁，没有用真正的文件锁，容错原则是"读写失败就当没有
//! 别的实例"，不会因为这个功能本身导致宠物卡住或崩溃。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::util::now_ms;

const STALE_MS: u128 = 2000; // 超过这个时长没更新的记录，视为对应实例已经退出/在动，忽略

fn registry_path() -> PathBuf {
    std::env::temp_dir().join("desktoppet_instances.json")
}

#[derive(Serialize, Deserialize, Clone, Copy)]
struct InstanceEntry {
    x: f64,
    y: f64,
    updated_ms: u128,
}

fn read_registry() -> HashMap<u32, InstanceEntry> {
    fs::read_to_string(registry_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 把自己当前的静止位置登记进共享文件，覆盖自己那一条，其他实例的条目原样保留。
/// 调用方负责节流(不需要每帧都写)，这里只是单纯的读-改-写。
pub fn publish_resting_position(x: f64, y: f64) {
    let mut map = read_registry();
    map.insert(
        std::process::id(),
        InstanceEntry {
            x,
            y,
            updated_ms: now_ms(),
        },
    );
    if let Ok(json) = serde_json::to_string(&map) {
        let _ = fs::write(registry_path(), json);
    }
}

/// 读取其他实例(排除自己、排除过期记录)当前登记的静止位置。
pub fn other_resting_positions() -> Vec<(f64, f64)> {
    let self_pid = std::process::id();
    let now = now_ms();
    read_registry()
        .into_iter()
        .filter(|(pid, entry)| *pid != self_pid && now.saturating_sub(entry.updated_ms) < STALE_MS)
        .map(|(_, entry)| (entry.x, entry.y))
        .collect()
}

/// 给定一个候选落脚点，判断是不是离别的实例已经登记的静止位置太近。
pub fn is_too_close(x: f64, y: f64, min_separation: f64, others: &[(f64, f64)]) -> bool {
    others
        .iter()
        .any(|(ox, oy)| (x - ox).hypot(y - oy) < min_separation)
}

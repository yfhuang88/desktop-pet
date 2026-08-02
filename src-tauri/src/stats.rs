//! 右键点宠物弹出/收起的系统状态窗口(CPU/RAM/电量)。完全通过
//! [`crate::physics::PetExtension`] 这个扩展点挂到核心状态机上，
//! 核心循环本身不知道"系统状态"这个功能的存在。

use sysinfo::System;
use tauri::{Manager, PhysicalPosition, Window};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

use crate::input::is_right_button_down;
use crate::physics::{PetExtension, TickContext, JUMP_SPACE, WINDOW_SIZE};
use crate::util::now_ms;

const STATS_WINDOW_WIDTH: f64 = 220.0; // 系统状态窗口宽度(px)，需与 tauri.conf.json 里 stats 窗口一致
const STATS_WINDOW_HEIGHT: f64 = 140.0; // 系统状态窗口高度(px)，需与 tauri.conf.json 里 stats 窗口一致
const STATS_GAP: f64 = 12.0; // 状态窗口与宠物本体之间的间隙(px)
const STATS_REFRESH_INTERVAL_MS: u128 = 1000; // 系统状态(CPU/RAM/电量)刷新间隔(ms)

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StatsPayload {
    cpu_percent: f32,
    ram_percent: f32,
    battery_percent: Option<f32>,
    on_ac_power: Option<bool>,
}

/// 读取电池电量(0-100)和是否接通电源；台式机等没有电池的设备返回 (None, None)
fn read_power_status() -> (Option<f32>, Option<bool>) {
    unsafe {
        let mut status = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut status).is_ok() {
            // BatteryFlag == 128 表示"没有电池"(台式机)，BatteryLifePercent == 255 表示"未知"
            if status.BatteryFlag == 128 || status.BatteryLifePercent == 255 {
                (None, None)
            } else {
                (
                    Some(status.BatteryLifePercent as f32),
                    Some(status.ACLineStatus == 1),
                )
            }
        } else {
            (None, None)
        }
    }
}

/// 计算状态窗口应该贴在宠物哪一侧：优先贴右侧，放不下就贴左侧；
/// 垂直方向与宠物窗口顶部对齐，并夹在当前显示器工作区内。
fn stats_anchor_position(
    pet_win_x: f64,
    pet_win_y: f64,
    area_x: f64,
    area_y: f64,
    area_w: f64,
    area_h: f64,
) -> (f64, f64) {
    let mut sx = pet_win_x + WINDOW_SIZE + STATS_GAP;
    if sx + STATS_WINDOW_WIDTH > area_x + area_w {
        sx = pet_win_x - STATS_GAP - STATS_WINDOW_WIDTH;
    }
    let max_sy = (area_y + area_h - STATS_WINDOW_HEIGHT).max(area_y);
    let sy = pet_win_y.max(area_y).min(max_sy);
    (sx, sy)
}

pub struct StatsExtension {
    stats_window: Window,
    sys: System,
    mouse_was_down_right: bool,
    right_press_active: bool,
    stats_open: bool,
    last_stats_emit_ms: u128,
}

impl StatsExtension {
    pub fn new(main_window: &Window) -> Self {
        Self {
            stats_window: main_window.get_window("stats").unwrap(),
            sys: System::new_all(),
            mouse_was_down_right: false,
            right_press_active: false,
            stats_open: false,
            last_stats_emit_ms: 0,
        }
    }
}

impl PetExtension for StatsExtension {
    fn should_freeze(&mut self, _pet_window: &Window, ctx: &TickContext) -> bool {
        // 右键点击切换系统状态窗口的显示/收起，判定逻辑跟左键点击一致：
        // 按下时鼠标落在宠物身上才算"按在宠物上"，松手时才真正触发切换。
        let mouse_down_now_right = is_right_button_down();
        if mouse_down_now_right && !self.mouse_was_down_right && ctx.is_hovered {
            self.right_press_active = true;
        }
        if !mouse_down_now_right && self.mouse_was_down_right {
            if self.right_press_active {
                self.stats_open = !self.stats_open;
                if self.stats_open {
                    let _ = self.stats_window.show();
                    self.last_stats_emit_ms = 0; // 强制窗口一打开就立刻刷新一次数据
                } else {
                    let _ = self.stats_window.hide();
                }
            }
            self.right_press_active = false;
        }
        self.mouse_was_down_right = mouse_down_now_right;

        self.stats_open
    }

    fn after_tick(&mut self, _pet_window: &Window, ctx: &TickContext) {
        if !self.stats_open {
            return;
        }

        // 宠物本身在状态窗口打开期间被冻结不动，但拖拽仍然允许移动它，
        // 所以每帧都重新贴一次位置，跟着宠物(而不是只在打开那一刻定死)。
        let win_x = ctx.pet_x - WINDOW_SIZE / 2.0;
        let win_y = ctx.pet_y - WINDOW_SIZE / 2.0 - JUMP_SPACE;
        let (sx, sy) = stats_anchor_position(win_x, win_y, ctx.area_x, ctx.area_y, ctx.area_w, ctx.area_h);
        let _ = self.stats_window.set_position(tauri::Position::Physical(PhysicalPosition {
            x: sx.round() as i32,
            y: sy.round() as i32,
        }));

        let now = now_ms();
        if now.saturating_sub(self.last_stats_emit_ms) >= STATS_REFRESH_INTERVAL_MS {
            self.last_stats_emit_ms = now;
            self.sys.refresh_cpu_usage();
            self.sys.refresh_memory();
            let cpu_percent = self.sys.global_cpu_usage();
            let total_mem = self.sys.total_memory();
            let ram_percent = if total_mem > 0 {
                (self.sys.used_memory() as f64 / total_mem as f64 * 100.0) as f32
            } else {
                0.0
            };
            let (battery_percent, on_ac_power) = read_power_status();
            let _ = self.stats_window.emit(
                "stats-update",
                StatsPayload {
                    cpu_percent,
                    ram_percent,
                    battery_percent,
                    on_ac_power,
                },
            );
        }
    }
}

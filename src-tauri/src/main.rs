#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rand::Rng;
use serde::Serialize;
use tauri::{
    AppHandle, CustomMenuItem, Manager, PhysicalPosition, SystemTray, SystemTrayEvent,
    SystemTrayMenu, SystemTrayMenuItem, Window,
};
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

// ------------------------- 可调参数(与 Electron 版保持一致) -------------------------
const WINDOW_SIZE: f64 = 170.0; // 宠物本体占用的正方形尺寸(px)
const JUMP_SPACE: f64 = 40.0; // 窗口顶部额外预留的空间(px)，用于点击弹跳动画
const SPRITE_W: f64 = 144.0; // 贴图实际渲染宽度(px)，跟 index.html 里的 img 尺寸对应
const SPRITE_H: f64 = 153.0; // 贴图实际渲染高度(px)
const STOP_DISTANCE: f64 = 20.0; // 距离"停靠点"小于这个值(px)时停下
const SIDE_OFFSET: f64 = 70.0; // 追逐时停靠点相对鼠标的水平偏移量(px)
const MAX_SPEED: f64 = 150.0; // 最大移动速度(px/秒)
const ACCELERATION: f64 = 350.0; // 加速度(px/秒^2)
const FRICTION: f64 = 6.0; // 到达目标附近后的刹车系数
const ANIM_FRAME_INTERVAL_MS: u128 = 180; // 走路动画切帧间隔(ms)
const TICK_INTERVAL_MS: u64 = 16; // 物理更新间隔(ms)，约等于 60fps
const FACE_FLIP_DEADZONE: f64 = 20.0; // 朝向翻转死区(px)

const ESCAPE_MIN_DISTANCE: f64 = 300.0; // 鼠标离宠物的距离超过这个值(px)，判定为甩开
const FAST_ESCAPE_SPEED: f64 = 1300.0; // 鼠标"远离宠物方向"的平均速度超过这个值(px/秒)，判定为甩开
const ESCAPE_SAMPLE_WINDOW_MS: u128 = 120; // 计算"甩开速度"的采样窗口(ms)
const RESUME_DISTANCE: f64 = 180.0; // 待机/溜达期间，鼠标进入这个距离(px)内重新开始追逐
const BOTTOM_IDLE_MIN_MS: f64 = 3000.0; // 贴底待机最短时间(ms)
const BOTTOM_IDLE_MAX_MS: f64 = 7000.0; // 贴底待机最长时间(ms)
const WANDER_RANGE_MIN: f64 = 80.0; // 贴底左右溜达最短距离(px)
const WANDER_RANGE_MAX: f64 = 500.0; // 贴底左右溜达最长距离(px)
const WANDER_MAX_SPEED: f64 = 50.0; // 贴底左右溜达时的最大速度(px/秒)
const WANDER_ACCELERATION: f64 = 150.0; // 贴底左右溜达时的加速度(px/秒^2)

const DRAG_HOLD_MS: f64 = 250.0; // 按住超过这个时长(ms)判定为"长按拖动"而不是点击
const DRAG_MOVE_THRESHOLD: f64 = 6.0; // 按住期间鼠标移动超过这个距离(px)也立刻判定为拖动
const DRAG_MAX_SPEED: f64 = 1400.0; // 拖拽时的最大跟随速度(px/秒)，越小越"沉"、越跟不上鼠标快速移动
// 拖拽跟随用"临界阻尼弹簧"模型(加速度 = K*位移差 - C*当前速度)，而不是"冲过去再刹车"，
// 这样无论鼠标怎么动都不会冲过目标点再弹回来。C 必须 >= 2*sqrt(K) 才不会有振荡/晃动，
// 这里特意留了约 15% 余量，抵消离散时间模拟(每帧16ms)带来的轻微超调风险。
const DRAG_SPRING_K: f64 = 500.0; // 弹簧强度，越大跟手越紧、追得越快
const DRAG_SPRING_DAMPING: f64 = 54.0; // 阻尼系数(临界值约为 2*sqrt(500)≈44.7)
// ------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Chase,
    Retreat,
    BottomIdle,
    BottomWalk,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PetState {
    walking: bool,
    dragging: bool,
    facing_left: bool,
    frame: &'static str,
}

fn get_cursor_pos() -> (f64, f64) {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    (point.x as f64, point.y as f64)
}

/// 返回距离给定屏幕坐标最近的显示器的"工作区"(不含任务栏) (x, y, width, height)
fn work_area_for(x: f64, y: f64) -> (f64, f64, f64, f64) {
    unsafe {
        let pt = POINT {
            x: x as i32,
            y: y as i32,
        };
        let hmonitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(hmonitor, &mut info);
        let r: RECT = info.rcWork;
        (
            r.left as f64,
            r.top as f64,
            (r.right - r.left) as f64,
            (r.bottom - r.top) as f64,
        )
    }
}

fn is_left_button_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 }
}

fn rand_range(min: f64, max: f64) -> f64 {
    let mut rng = rand::thread_rng();
    min + rng.gen::<f64>() * (max - min)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn spawn_physics_loop(window: Window) {
    thread::spawn(move || {
        let (cx, cy) = get_cursor_pos();
        let (ax, ay, aw, ah) = work_area_for(cx, cy);

        let mut pet_x = ax + aw / 2.0;
        let mut pet_y = ay + ah / 2.0;
        let mut vel_x = 0.0_f64;
        let mut vel_y = 0.0_f64;
        let mut facing_left = false;
        let mut is_walking;
        let mut mode = Mode::Chase;
        let mut idle_timer_ms = 0.0_f64;
        let mut wander_target_x = pet_x;
        let mut is_hovered = false;

        // 拖拽相关状态：长按宠物本体可以把它拖到屏幕任意位置(包括另一块屏幕)，
        // 松手后自动进入"回到底部待机/溜达"流程；短按(没有拖动过)则触发跳跃。
        let mut mouse_was_down = false;
        let mut press_start: Option<Instant> = None;
        let mut press_start_cursor = (0.0_f64, 0.0_f64);
        let mut is_dragging = false;
        let mut drag_offset = (0.0_f64, 0.0_f64);

        let mut escape_sample_cursor = (cx, cy);
        let mut escape_sample_time = now_ms();

        let walk_frames = ["walk1", "walk2", "walk3", "walk2"];
        let mut walk_frame_index: usize = 0;
        let mut last_anim_time = now_ms();
        let mut last_tick = Instant::now();

        // 设置初始位置(屏幕中央)，再显示窗口，避免出现在默认(0,0)位置的闪烁
        let _ = window.set_position(tauri::Position::Physical(PhysicalPosition {
            x: (pet_x - WINDOW_SIZE / 2.0).round() as i32,
            y: (pet_y - WINDOW_SIZE / 2.0 - JUMP_SPACE).round() as i32,
        }));
        let _ = window.set_ignore_cursor_events(true);
        let _ = window.show();

        loop {
            let frame_start = Instant::now();
            let dt = (frame_start - last_tick).as_secs_f64().min(0.05);
            last_tick = frame_start;

            let (cursor_x, cursor_y) = get_cursor_pos();
            let raw_dx = cursor_x - pet_x;
            let raw_dy = cursor_y - pet_y;
            let raw_dist = raw_dx.hypot(raw_dy).max(1.0);

            let (area_x, area_y, area_w, area_h) = work_area_for(pet_x, pet_y);
            let bottom_y = area_y + area_h - WINDOW_SIZE / 2.0;

            // 悬停判定：鼠标是否落在宠物贴图的实际渲染区域内(而不是整个窗口矩形)
            let win_x = pet_x - WINDOW_SIZE / 2.0;
            let win_y = pet_y - WINDOW_SIZE / 2.0 - JUMP_SPACE;
            let sprite_left = win_x + (WINDOW_SIZE - SPRITE_W) / 2.0;
            let sprite_right = sprite_left + SPRITE_W;
            let sprite_bottom = win_y + WINDOW_SIZE + JUMP_SPACE;
            let sprite_top = sprite_bottom - SPRITE_H;
            let now_hovered = cursor_x >= sprite_left
                && cursor_x <= sprite_right
                && cursor_y >= sprite_top
                && cursor_y <= sprite_bottom;

            if now_hovered != is_hovered {
                is_hovered = now_hovered;
                // 悬停时关闭"点击穿透"，让点击能落在宠物本体上；离开后恢复穿透
                let _ = window.set_ignore_cursor_events(!is_hovered);
            }

            // 长按/拖拽/点击 判定：只在鼠标左键刚按下时且落在宠物身上才开始追踪，
            // 按住超过 DRAG_HOLD_MS 或移动超过 DRAG_MOVE_THRESHOLD 就判定为"拖拽"，
            // 否则松手时判定为一次"点击"，触发跳跃(通过 pet-jump 事件通知前端播放动画)。
            let mouse_down_now = is_left_button_down();
            if mouse_down_now && !mouse_was_down && is_hovered {
                press_start = Some(Instant::now());
                press_start_cursor = (cursor_x, cursor_y);
            }
            if mouse_down_now {
                if let Some(start) = press_start {
                    if !is_dragging {
                        let held_ms = start.elapsed().as_secs_f64() * 1000.0;
                        let moved = (cursor_x - press_start_cursor.0).hypot(cursor_y - press_start_cursor.1);
                        if held_ms > DRAG_HOLD_MS || moved > DRAG_MOVE_THRESHOLD {
                            is_dragging = true;
                            drag_offset = (pet_x - cursor_x, pet_y - cursor_y);
                            vel_x = 0.0;
                            vel_y = 0.0;
                        }
                    }
                }
            }
            if !mouse_down_now && mouse_was_down {
                if is_dragging {
                    is_dragging = false;
                    // 放下后自动回到屏幕底部待机，再进入左右溜达循环
                    mode = Mode::Retreat;
                } else if press_start.is_some() {
                    let _ = window.emit("pet-jump", ());
                }
                press_start = None;
            }
            mouse_was_down = mouse_down_now;

            if is_dragging {
                // 拖拽中：用临界阻尼弹簧跟随"鼠标+抓取偏移"这个目标点，而不是
                // "全速冲过去、快到了再刹车"。弹簧模型在任何距离下都同时有拉力
                // (朝目标)和阻力(跟当前速度成正比)，数学上保证不会冲过头再弹回来，
                // 拖着走时仍然会有一点跟不上的滞后感("重量感")，停下来也不会晃。
                let drag_target_x = cursor_x + drag_offset.0;
                let drag_target_y = cursor_y + drag_offset.1;
                let ddx = drag_target_x - pet_x;
                let ddy = drag_target_y - pet_y;

                let ax = ddx * DRAG_SPRING_K - vel_x * DRAG_SPRING_DAMPING;
                let ay = ddy * DRAG_SPRING_K - vel_y * DRAG_SPRING_DAMPING;
                vel_x += ax * dt;
                vel_y += ay * dt;

                // 鼠标猛地移动一大段距离时限速，避免瞬间大幅位移显得"飘"
                let speed = vel_x.hypot(vel_y);
                if speed > DRAG_MAX_SPEED {
                    vel_x = vel_x / speed * DRAG_MAX_SPEED;
                    vel_y = vel_y / speed * DRAG_MAX_SPEED;
                }

                pet_x += vel_x * dt;
                pet_y += vel_y * dt;
                is_walking = false;
            } else {
                if mode != Mode::Chase {
                    // 待机/溜达期间，鼠标靠近就重新开始追逐
                    if raw_dist < RESUME_DISTANCE {
                        mode = Mode::Chase;
                    }
                } else {
                    // 判定"甩开"：每隔 ESCAPE_SAMPLE_WINDOW_MS 采样一次鼠标位移，
                    // 算出这段时间内沿"背离宠物"方向的平均速度。
                    // 距离够远或者平均速度够快，满足任意一个条件就放弃追逐。
                    let now = now_ms();
                    let elapsed = now.saturating_sub(escape_sample_time);
                    if elapsed >= ESCAPE_SAMPLE_WINDOW_MS {
                        let window_seconds = elapsed as f64 / 1000.0;
                        let avg_vel_x = (cursor_x - escape_sample_cursor.0) / window_seconds;
                        let avg_vel_y = (cursor_y - escape_sample_cursor.1) / window_seconds;
                        let away_speed = (avg_vel_x * raw_dx + avg_vel_y * raw_dy) / raw_dist;
                        if raw_dist > ESCAPE_MIN_DISTANCE || away_speed > FAST_ESCAPE_SPEED {
                            mode = Mode::Retreat;
                        }
                        escape_sample_cursor = (cursor_x, cursor_y);
                        escape_sample_time = now;
                    }
                }

                let mut target_x = pet_x;
                let mut target_y = pet_y;
                let mut should_seek = true;

                if is_hovered {
                    // 鼠标悬停在宠物身上，强制停下，不追逐任何目标
                    should_seek = false;
                } else {
                    match mode {
                        Mode::Chase => {
                            // 停靠点不是鼠标本身，而是鼠标水平方向侧边的一个偏移点，
                            // 这样宠物靠近后停下时不会正好挡在鼠标指针/桌面图标上面。
                            let side = if pet_x <= cursor_x { -1.0 } else { 1.0 };
                            target_x = cursor_x + side * SIDE_OFFSET;
                            target_y = cursor_y;
                        }
                        Mode::Retreat => {
                            // 直接垂直走回屏幕底部待机，横向位置不变
                            target_x = pet_x;
                            target_y = bottom_y;
                            if (target_x - pet_x).hypot(target_y - pet_y) < STOP_DISTANCE {
                                mode = Mode::BottomIdle;
                                idle_timer_ms = rand_range(BOTTOM_IDLE_MIN_MS, BOTTOM_IDLE_MAX_MS);
                            }
                        }
                        Mode::BottomIdle => {
                            target_x = pet_x;
                            target_y = bottom_y;
                            should_seek = false;
                            idle_timer_ms -= dt * 1000.0;
                            if idle_timer_ms <= 0.0 {
                                mode = Mode::BottomWalk;
                                let range = rand_range(WANDER_RANGE_MIN, WANDER_RANGE_MAX);
                                let dir = if rand::thread_rng().gen::<bool>() {
                                    1.0
                                } else {
                                    -1.0
                                };
                                wander_target_x = (pet_x + dir * range)
                                    .max(area_x + WINDOW_SIZE / 2.0)
                                    .min(area_x + area_w - WINDOW_SIZE / 2.0);
                            }
                        }
                        Mode::BottomWalk => {
                            target_x = wander_target_x;
                            target_y = bottom_y;
                            if (target_x - pet_x).hypot(target_y - pet_y) < STOP_DISTANCE {
                                mode = Mode::BottomIdle;
                                idle_timer_ms = rand_range(BOTTOM_IDLE_MIN_MS, BOTTOM_IDLE_MAX_MS);
                            }
                        }
                    }
                }

                let dx = target_x - pet_x;
                let dy = target_y - pet_y;
                let dist = dx.hypot(dy);

                // 朝向：只有"追逐鼠标"或"被鼠标悬停按住"时才看向鼠标真实位置；
                // 被甩开/走回底部/贴底待机/溜达时，朝向跟随自己的移动方向，不再跟着鼠标转。
                if is_hovered || mode == Mode::Chase {
                    if raw_dx.abs() > FACE_FLIP_DEADZONE {
                        facing_left = raw_dx < 0.0;
                    }
                } else if dx.abs() > FACE_FLIP_DEADZONE {
                    facing_left = dx < 0.0;
                }

                // 贴底左右溜达时用慢速参数，营造"慢悠悠"的感觉；其余状态保持原速度
                let (max_speed, acceleration) = if mode == Mode::BottomWalk {
                    (WANDER_MAX_SPEED, WANDER_ACCELERATION)
                } else {
                    (MAX_SPEED, ACCELERATION)
                };

                if should_seek && dist > STOP_DISTANCE {
                    let nx = dx / dist;
                    let ny = dy / dist;
                    vel_x += nx * acceleration * dt;
                    vel_y += ny * acceleration * dt;

                    let speed = vel_x.hypot(vel_y);
                    if speed > max_speed {
                        vel_x = vel_x / speed * max_speed;
                        vel_y = vel_y / speed * max_speed;
                    }
                    is_walking = true;
                } else {
                    let damp = (FRICTION * dt).min(1.0);
                    vel_x -= vel_x * damp;
                    vel_y -= vel_y * damp;
                    is_walking = vel_x.hypot(vel_y) > 5.0;
                }

                pet_x += vel_x * dt;
                pet_y += vel_y * dt;
            }

            // 限制宠物停留在当前显示器工作区内，避免跑出屏幕(拖拽时也一样，
            // 用拖拽/移动后的最新位置重新判定所在显示器，避免跨屏拖动时卡在边界)
            let (clamp_x, clamp_y, clamp_w, clamp_h) = work_area_for(pet_x, pet_y);
            pet_x = pet_x
                .max(clamp_x + WINDOW_SIZE / 2.0)
                .min(clamp_x + clamp_w - WINDOW_SIZE / 2.0);
            pet_y = pet_y
                .max(clamp_y + WINDOW_SIZE / 2.0 + JUMP_SPACE)
                .min(clamp_y + clamp_h - WINDOW_SIZE / 2.0);

            let _ = window.set_position(tauri::Position::Physical(PhysicalPosition {
                x: (pet_x - WINDOW_SIZE / 2.0).round() as i32,
                y: (pet_y - WINDOW_SIZE / 2.0 - JUMP_SPACE).round() as i32,
            }));

            let now = now_ms();
            if now.saturating_sub(last_anim_time) >= ANIM_FRAME_INTERVAL_MS {
                last_anim_time = now;
                if is_walking {
                    walk_frame_index = (walk_frame_index + 1) % walk_frames.len();
                } else {
                    walk_frame_index = 0;
                }
            }

            let frame = if is_walking {
                walk_frames[walk_frame_index]
            } else {
                "idle"
            };
            let _ = window.emit(
                "pet-state",
                PetState {
                    walking: is_walking,
                    dragging: is_dragging,
                    facing_left,
                    frame,
                },
            );

            let elapsed_frame = frame_start.elapsed();
            let target_dur = Duration::from_millis(TICK_INTERVAL_MS);
            if elapsed_frame < target_dur {
                thread::sleep(target_dur - elapsed_frame);
            }
        }
    });
}

fn toggle_visibility(app: &AppHandle) {
    if let Some(window) = app.get_window("main") {
        let visible = window.is_visible().unwrap_or(false);
        if visible {
            let _ = window.hide();
        } else {
            let _ = window.show();
        }
        let _ = app
            .tray_handle()
            .get_item("toggle")
            .set_title(if visible { "显示" } else { "隐藏" });
    }
}

fn main() {
    // 显式声明"每个显示器独立感知 DPI"，避免在不同缩放比例的多显示器环境下，
    // Windows 对 GetCursorPos 等坐标做虚拟化换算，导致跨屏拖拽时坐标对不上、
    // 卡在屏幕边界拖不过去的问题。必须在创建任何窗口之前调用。
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let toggle = CustomMenuItem::new("toggle".to_string(), "隐藏");
    let quit = CustomMenuItem::new("quit".to_string(), "退出");
    let tray_menu = SystemTrayMenu::new()
        .add_item(toggle)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(quit);
    let tray = SystemTray::new().with_menu(tray_menu);

    tauri::Builder::default()
        .system_tray(tray)
        .on_system_tray_event(|app, event| match event {
            SystemTrayEvent::LeftClick { .. } => {
                toggle_visibility(app);
            }
            SystemTrayEvent::MenuItemClick { id, .. } => match id.as_str() {
                "toggle" => toggle_visibility(app),
                "quit" => {
                    std::process::exit(0);
                }
                _ => {}
            },
            _ => {}
        })
        .setup(|app| {
            let window = app.get_window("main").unwrap();
            spawn_physics_loop(window);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

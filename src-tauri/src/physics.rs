//! 核心状态机(baseline-simple 里的那一套)：追逐/待机/溜达/拖拽/跳跃。
//! 不认识任何具体功能扩展(系统状态弹窗等)，只通过 [`PetExtension`] 这个扩展点
//! 给外部功能一个"每帧介入"的机会——外部功能自己决定要不要冻结宠物、要不要
//! 跟着宠物定位自己的窗口，核心循环本身对这些功能一无所知。

use std::thread;
use std::time::{Duration, Instant};

use rand::Rng;
use serde::Serialize;
use tauri::{PhysicalPosition, Window};

use crate::input::{get_cursor_pos, is_left_button_down, work_area_for};
use crate::instances::{is_too_close, other_resting_positions, publish_resting_position};
use crate::util::{now_ms, rand_range};

// 默认 idle 贴图的原始字节，编译时直接打进 exe 里，不依赖运行时的文件路径。
const IDLE_PNG_BYTES: &[u8] = include_bytes!("../../src/assets/idle.png");

/// 用 idle 贴图的透明通道当"命中遮罩"：判断鼠标是不是压在贴图实际画出来的、
/// 不透明的部分上，而不是单纯落在贴图的矩形范围内——矩形框重叠但人像本身没
/// 重叠时，能分清鼠标到底点中了哪一个。用 idle 这张图的形状近似代表所有帧
/// (走路/拖拽帧的轮廓大体一致)，不需要给每一帧都单独判断，够用且简单。
struct HitMask {
    img: image::RgbaImage,
}

impl HitMask {
    fn load() -> Option<Self> {
        image::load_from_memory(IDLE_PNG_BYTES)
            .ok()
            .map(|img| HitMask { img: img.to_rgba8() })
    }

    /// rel_x/rel_y 是贴图内的相对坐标(0.0=左/上边缘，1.0=右/下边缘)。
    fn is_opaque_at(&self, rel_x: f64, rel_y: f64) -> bool {
        if !(0.0..1.0).contains(&rel_x) || !(0.0..1.0).contains(&rel_y) {
            return false;
        }
        let px = ((rel_x * self.img.width() as f64) as u32).min(self.img.width().saturating_sub(1));
        let py = ((rel_y * self.img.height() as f64) as u32).min(self.img.height().saturating_sub(1));
        self.img.get_pixel(px, py)[3] > 10 // alpha 阈值，容忍边缘抗锯齿的半透明像素
    }
}

// ------------------------- 可调参数(与 Electron 版保持一致) -------------------------
pub const WINDOW_SIZE: f64 = 170.0; // 宠物本体占用的正方形尺寸(px)
pub const JUMP_SPACE: f64 = 40.0; // 窗口顶部额外预留的空间(px)，用于点击弹跳动画
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

// 多开实例互相避让：只影响"选哪个落脚点"，走路/溜达途中互相穿过完全不受影响。
const MIN_INSTANCE_SEPARATION: f64 = 200.0; // 贴底待机/溜达的落脚点，至少要隔多远(px)
// 追逐时只有"鼠标左边"和"鼠标右边"两个候选停靠点，彼此最多隔 2*SIDE_OFFSET，
// 用 MIN_INSTANCE_SEPARATION 这种"宽敞"场景的阈值永远达不到、会导致每帧都在翻边。
// 这里改成"别跟别人在同一侧"这个能真正达到的目标，阈值定得比 SIDE_OFFSET 小一点。
const CHASE_SIDE_MIN_SEPARATION: f64 = SIDE_OFFSET * 0.8;
const PUBLISH_INTERVAL_MS: u128 = 500; // 静止状态下，多久把自己的位置登记一次(节流，不用每帧写)
const WANDER_RETRY_ATTEMPTS: u32 = 5; // 溜达目的地跟别人冲突时，最多重新随机几次
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

/// 每帧传给功能扩展的只读快照：宠物当前位置、所在显示器工作区、是否被悬停。
pub struct TickContext {
    pub pet_x: f64,
    pub pet_y: f64,
    pub area_x: f64,
    pub area_y: f64,
    pub area_w: f64,
    pub area_h: f64,
    pub is_hovered: bool,
}

/// 核心状态机对外暴露的唯一扩展点。核心循环每帧调用两次：
/// - `should_freeze`：移动前，功能扩展可以据此让宠物这一帧不自主移动(拖拽不受影响)；
/// - `after_tick`：宠物本帧最终位置确定后，用于跟随定位自己的附属窗口、按需轮询数据等。
/// 默认实现都是空操作，扩展只需要实现自己关心的那一个。
pub trait PetExtension: Send {
    fn should_freeze(&mut self, _pet_window: &Window, _ctx: &TickContext) -> bool {
        false
    }

    fn after_tick(&mut self, _pet_window: &Window, _ctx: &TickContext) {}
}

/// 在贴底的这条线上随机挑一个溜达目的地，尽量避开别的实例已登记的静止位置。
/// 试几次都冲突就放弃、直接接受最后一次算出来的结果，避免死循环卡住。
fn pick_wander_target(
    pet_x: f64,
    area_x: f64,
    area_w: f64,
    bottom_y: f64,
    others: &[(f64, f64)],
) -> f64 {
    let mut candidate = pet_x;
    for _ in 0..WANDER_RETRY_ATTEMPTS {
        let range = rand_range(WANDER_RANGE_MIN, WANDER_RANGE_MAX);
        let dir = if rand::thread_rng().gen::<bool>() { 1.0 } else { -1.0 };
        candidate = (pet_x + dir * range)
            .max(area_x + WINDOW_SIZE / 2.0)
            .min(area_x + area_w - WINDOW_SIZE / 2.0);
        if !is_too_close(candidate, bottom_y, MIN_INSTANCE_SEPARATION, others) {
            return candidate;
        }
    }
    candidate
}

pub fn spawn_physics_loop(window: Window, mut extensions: Vec<Box<dyn PetExtension>>) {
    thread::spawn(move || {
        let hit_mask = HitMask::load();

        let (cx, cy) = get_cursor_pos();
        let (ax, ay, aw, ah) = work_area_for(cx, cy);

        let mut pet_x = ax + aw / 2.0;
        let mut pet_y = ay + ah / 2.0;
        let mut vel_x = 0.0_f64;
        let mut vel_y = 0.0_f64;
        let mut facing_left = false;
        let mut is_walking;
        // 追逐时停靠在鼠标哪一侧，只在"刚开始追逐"这一刻决定一次，追逐过程中固定
        // 不变(除非被冲突检测翻转)，不要每帧都根据当前位置重新计算——否则一旦
        // 翻转的结果在下一帧又被重新算回原来那一侧，会在两个点之间来回抖动，
        // 永远稳定不到任何一个点上。
        let mut chase_side: f64 = if pet_x <= cx { -1.0 } else { 1.0 };
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
        let mut last_publish_time: u128 = 0;

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

            // 悬停判定：鼠标是否落在宠物贴图的实际渲染区域内，而且还落在贴图
            // 本身画出来的不透明像素上(不是矩形范围内的透明留白)——多开时，
            // 几个宠物的矩形范围可能会重叠，但只有真的点在人像上才算数，
            // 避免鼠标明明只点中了一个，其余因为方框重叠也一起被选中。
            let win_x = pet_x - WINDOW_SIZE / 2.0;
            let win_y = pet_y - WINDOW_SIZE / 2.0 - JUMP_SPACE;
            let sprite_left = win_x + (WINDOW_SIZE - SPRITE_W) / 2.0;
            let sprite_right = sprite_left + SPRITE_W;
            let sprite_bottom = win_y + WINDOW_SIZE + JUMP_SPACE;
            let sprite_top = sprite_bottom - SPRITE_H;
            let in_bbox = cursor_x >= sprite_left
                && cursor_x <= sprite_right
                && cursor_y >= sprite_top
                && cursor_y <= sprite_bottom;
            let now_hovered = in_bbox
                && match &hit_mask {
                    Some(mask) => {
                        let rel_x = (cursor_x - sprite_left) / SPRITE_W;
                        let rel_y = (cursor_y - sprite_top) / SPRITE_H;
                        mask.is_opaque_at(rel_x, rel_y)
                    }
                    // 遮罩没能读出来(比如内置图片打不开)，就退回矩形判定，
                    // 好歹还能点，不会因为这个功能而彻底点不到宠物。
                    None => true,
                };

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

            // 移动前给功能扩展一个机会：任意一个扩展要求冻结，这一帧就不自主移动
            // (拖拽不受影响，仍然可以把宠物拖着走)。
            let pre_ctx = TickContext {
                pet_x,
                pet_y,
                area_x,
                area_y,
                area_w,
                area_h,
                is_hovered,
            };
            let mut freeze_movement = false;
            for ext in extensions.iter_mut() {
                if ext.should_freeze(&window, &pre_ctx) {
                    freeze_movement = true;
                }
            }

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
                    // 待机/溜达期间，鼠标靠近就重新开始追逐；重新决定一次停靠在哪一侧。
                    if raw_dist < RESUME_DISTANCE {
                        mode = Mode::Chase;
                        chase_side = if pet_x <= cursor_x { -1.0 } else { 1.0 };
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

                if is_hovered || freeze_movement {
                    // 鼠标悬停在宠物身上，或某个功能扩展要求冻结，强制停下，不追逐任何目标
                    should_seek = false;
                } else {
                    // 多开时用来避让别的实例已登记的静止位置，只在"选落脚点"的
                    // 这几个时刻查询，不影响移动路径本身。
                    let others = other_resting_positions();

                    match mode {
                        Mode::Chase => {
                            // 停靠点不是鼠标本身，而是鼠标水平方向侧边的一个偏移点，
                            // 这样宠物靠近后停下时不会正好挡在鼠标指针/桌面图标上面。
                            // 用持久的 chase_side(不是每帧重新按当前位置计算)，
                            // 避免翻转之后下一帧又被算回原来那一侧、来回抖动。
                            let mut candidate_x = cursor_x + chase_side * SIDE_OFFSET;
                            // 首选边已经被别的实例占了(比如都从左边靠近同一个鼠标)，
                            // 就永久翻到另一边，避免两个实例算出一模一样的停靠点。
                            if is_too_close(candidate_x, cursor_y, CHASE_SIDE_MIN_SEPARATION, &others) {
                                chase_side = -chase_side;
                                candidate_x = cursor_x + chase_side * SIDE_OFFSET;
                            }
                            target_x = candidate_x;
                            target_y = cursor_y;
                        }
                        Mode::Retreat => {
                            // 直接垂直走回屏幕底部待机，横向位置不变
                            target_x = pet_x;
                            target_y = bottom_y;
                            if (target_x - pet_x).hypot(target_y - pet_y) < STOP_DISTANCE {
                                if is_too_close(target_x, target_y, MIN_INSTANCE_SEPARATION, &others) {
                                    // 刚好落在别的实例待机的地方，别真的停下，
                                    // 直接转成"溜达"走开一段距离再待机。
                                    mode = Mode::BottomWalk;
                                    wander_target_x = pick_wander_target(
                                        pet_x, area_x, area_w, bottom_y, &others,
                                    );
                                } else {
                                    mode = Mode::BottomIdle;
                                    idle_timer_ms = rand_range(BOTTOM_IDLE_MIN_MS, BOTTOM_IDLE_MAX_MS);
                                }
                            }
                        }
                        Mode::BottomIdle => {
                            target_x = pet_x;
                            target_y = bottom_y;
                            should_seek = false;
                            idle_timer_ms -= dt * 1000.0;
                            if idle_timer_ms <= 0.0 {
                                mode = Mode::BottomWalk;
                                wander_target_x = pick_wander_target(
                                    pet_x, area_x, area_w, bottom_y, &others,
                                );
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

                // 追逐鼠标时，目标点直接就是鼠标(侧边偏移后的)坐标，但鼠标可能停在
                // 宠物物理上够不到的地方(屏幕最顶/底那一小段，因为有 JUMP_SPACE 和
                // 半个窗口宽度的边界限制)。到达判定要用"宠物实际够得到的位置"来算，
                // 不然距离永远大于 STOP_DISTANCE，会一直卡在边界原地全速"用力"、
                // 走路动画也停不下来。这里把目标点也提前钳制到跟宠物本体同样的范围。
                target_x = target_x
                    .max(area_x + WINDOW_SIZE / 2.0)
                    .min(area_x + area_w - WINDOW_SIZE / 2.0);
                target_y = target_y
                    .max(area_y + WINDOW_SIZE / 2.0 + JUMP_SPACE)
                    .min(area_y + area_h - WINDOW_SIZE / 2.0);

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

            // 静止(没在走路、也没在被拖拽)时，定期把自己的位置登记出去，让别的
            // 实例挑落脚点时能避开这里；移动/拖拽途中不登记，节流到每 500ms 一次。
            if !is_walking && !is_dragging {
                let now = now_ms();
                if now.saturating_sub(last_publish_time) >= PUBLISH_INTERVAL_MS {
                    publish_resting_position(pet_x, pet_y);
                    last_publish_time = now;
                }
            }

            // 宠物本帧最终位置已经确定，给功能扩展一个机会跟随定位/轮询数据。
            let post_ctx = TickContext {
                pet_x,
                pet_y,
                area_x: clamp_x,
                area_y: clamp_y,
                area_w: clamp_w,
                area_h: clamp_h,
                is_hovered,
            };
            for ext in extensions.iter_mut() {
                ext.after_tick(&window, &post_ctx);
            }

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

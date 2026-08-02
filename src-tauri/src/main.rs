#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod input;
mod physics;
mod stats;
mod util;

use tauri::{
    AppHandle, CustomMenuItem, Manager, SystemTray, SystemTrayEvent, SystemTrayMenu,
    SystemTrayMenuItem,
};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

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
            let stats_extension = stats::StatsExtension::new(&window);
            let extensions: Vec<Box<dyn physics::PetExtension>> = vec![Box::new(stats_extension)];
            physics::spawn_physics_loop(window, extensions);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, Window, WindowEvent};
use tauri_plugin_notification::NotificationExt;

use crate::commands::AppState;

const MAIN_WINDOW_LABEL: &str = "main";
const OPEN_MENU_ID: &str = "tray-open";
const QUIT_MENU_ID: &str = "tray-quit";

#[derive(Default)]
struct DesktopState {
    hidden_notification_shown: AtomicBool,
}

pub fn setup(app: &mut App) -> tauri::Result<()> {
    app.manage(DesktopState::default());

    let open_item = MenuItem::with_id(app, OPEN_MENU_ID, "打开 SaveLink", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, QUIT_MENU_ID, "退出 SaveLink", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open_item, &separator, &quit_item])?;

    let mut tray = TrayIconBuilder::new()
        .tooltip("SaveLink")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            OPEN_MENU_ID => show_main_window(app),
            QUIT_MENU_ID => {
                if let Some(state) = app.try_state::<AppState>() {
                    state.save_discovery.shutdown();
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            let should_show = matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            );
            if should_show {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn handle_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW_LABEL {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        if let Err(error) = window.hide() {
            eprintln!("隐藏 SaveLink 主窗口失败: {error}");
            return;
        }
        notify_hidden_once(window.app_handle());
    }
}

pub fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

fn notify_hidden_once(app: &AppHandle) {
    let state = app.state::<DesktopState>();
    if state
        .hidden_notification_shown
        .swap(true, Ordering::Relaxed)
    {
        return;
    }

    if let Err(error) = app
        .notification()
        .builder()
        .title("SaveLink 正在后台运行")
        .body("可通过 Windows 系统托盘重新打开或退出 SaveLink。")
        .show()
    {
        state
            .hidden_notification_shown
            .store(false, Ordering::Relaxed);
        eprintln!("显示 SaveLink 托盘通知失败: {error}");
    }
}

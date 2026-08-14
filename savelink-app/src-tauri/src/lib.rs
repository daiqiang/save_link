// Tauri 应用入口。命令实现见 commands 模块。
mod auto_backup;
mod commands;
mod desktop;
mod oauth_config;

use commands::AppState;
use savelink_core::service::startup_self_check;
use std::path::PathBuf;
use tauri::Manager;

const TEST_DATA_DIR_ENV: &str = "SAVELINK_TEST_DATA_DIR";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            desktop::show_main_window(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            desktop::setup(app)?;
            // 在系统应用数据目录下初始化数据库与仓库（savelink.db + repository/）。
            let default_data_dir = app.path().app_data_dir().expect("无法获取应用数据目录");
            let (data_dir, profile_label) = configured_data_dir(default_data_dir);
            let state = match profile_label {
                Some(label) => AppState::init_with_profile(&data_dir, Some(label)),
                None => AppState::init(&data_dir),
            }
            .expect("初始化数据层失败");
            // 清理上次异常中断留下的 Writing 半成品快照，再开放前端命令。
            startup_self_check(&state.repo, &state.store).expect("启动自检失败");
            app.manage(state);
            auto_backup::start(app.handle().clone());
            Ok(())
        })
        .on_window_event(desktop::handle_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::list_games,
            commands::get_repository_path,
            commands::get_app_info,
            commands::get_auto_backup_settings,
            commands::set_auto_backup_enabled,
            commands::get_baidu_connection_status,
            commands::connect_baidu,
            commands::upload_snapshot_to_baidu,
            commands::discover_baidu_snapshots,
            commands::receive_baidu_snapshot,
            commands::list_snapshots,
            commands::scan_path,
            commands::scan_steam_games,
            commands::scan_desmume_games,
            commands::register_desmume_game,
            commands::add_game,
            commands::update_game,
            commands::create_snapshot,
            commands::update_snapshot_meta,
            commands::delete_snapshot,
            commands::delete_game,
            commands::restore_snapshot,
            commands::restore_snapshot_with_choice,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn configured_data_dir(default_data_dir: PathBuf) -> (PathBuf, Option<String>) {
    let Some(value) = std::env::var_os(TEST_DATA_DIR_ENV).filter(|value| !value.is_empty()) else {
        return (default_data_dir, None);
    };
    let path = PathBuf::from(value);
    assert!(path.is_absolute(), "{TEST_DATA_DIR_ENV} 必须是绝对路径");
    assert_ne!(
        path, default_data_dir,
        "{TEST_DATA_DIR_ENV} 不能指向正式数据目录"
    );
    (path, Some("设备 B 隔离测试".into()))
}

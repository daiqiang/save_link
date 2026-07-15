// Tauri 应用入口。命令实现见 commands 模块。
mod commands;
mod oauth_config;

use commands::AppState;
use savelink_core::service::startup_self_check;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 在系统应用数据目录下初始化数据库与仓库（savelink.db + repository/）。
            let data_dir = app.path().app_data_dir().expect("无法获取应用数据目录");
            let state = AppState::init(&data_dir).expect("初始化数据层失败");
            // 清理上次异常中断留下的 Writing 半成品快照，再开放前端命令。
            startup_self_check(&state.repo, &state.store).expect("启动自检失败");
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_games,
            commands::get_repository_path,
            commands::get_app_info,
            commands::get_baidu_connection_status,
            commands::connect_baidu,
            commands::list_snapshots,
            commands::scan_path,
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

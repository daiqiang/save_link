// Tauri 应用入口。命令实现见 commands 模块。
mod commands;

use commands::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 在系统应用数据目录下初始化数据库与仓库（savelink.db + repository/）。
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("无法获取应用数据目录");
            let state = AppState::init(&data_dir).expect("初始化数据层失败");
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_games,
            commands::list_snapshots,
            commands::scan_path,
            commands::add_game,
            commands::create_snapshot,
            commands::update_snapshot_meta,
            commands::delete_snapshot,
            commands::restore_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

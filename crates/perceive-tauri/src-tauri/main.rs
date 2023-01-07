#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

pub mod app_state;

#[tauri::command]
fn run_search() {}

fn main() {
    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![run_search])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

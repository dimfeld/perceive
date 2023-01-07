#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::sync::Arc;

use app_state::AppState;
use parking_lot::Mutex;
use perceive_core::db::Database;
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

pub mod app_state;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "status")]
pub enum LoadState {
    Loading,
    Loaded,
    Error { error: String },
}

#[tauri::command]
fn load_status(status: State<Arc<Mutex<LoadState>>>) -> LoadState {
    let value = status.lock();
    value.clone()
}

fn main() {
    let database = Database::new(None);

    tauri::Builder::default()
        .setup(|app| {
            let app_state = app.state::<AppState>();

            let database = Database::new(None)?;
            app.manage(database.clone());

            let load_status = Arc::new(Mutex::new(LoadState::Loading));
            app.manage(load_status.clone());

            let window = app.get_window("main").unwrap();

            let model_type = app_state.model_type;
            let model_rx = app_state
                .model
                .rebuild(move || perceive_core::model::Model::new_pretrained(model_type));

            let searcher_rx = app_state.build_searcher(database);

            std::thread::spawn(move || {
                let result = match (model_rx.recv(), searcher_rx.recv()) {
                    (Ok(Ok(_)), Ok(Ok(_))) => LoadState::Loaded,
                    (Ok(Err(e)), _) => LoadState::Error {
                        error: e.to_string(),
                    },
                    (Err(e), _) => LoadState::Error {
                        error: e.to_string(),
                    },
                    (_, Ok(Err(e))) => LoadState::Error {
                        error: e.to_string(),
                    },
                    (_, Err(e)) => LoadState::Error {
                        error: e.to_string(),
                    },
                };

                {
                    let mut value = load_status.lock();
                    *value = result.clone();
                }
                window.emit("load-status", Some(result)).ok();
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![load_status])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

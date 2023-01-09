#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::sync::Arc;

use app_state::AppState;
use eyre::eyre;
use parking_lot::Mutex;
use perceive_core::{db::Database, search::SearchItem, Item};
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

#[tauri::command]
fn search(query: String, db: State<Database>, state: State<AppState>) -> Result<Vec<Item>, String> {
    let searcher = state.get_searcher().map_err(|e| e.to_string())?;
    let model = state.get_model().map_err(|e| e.to_string())?;
    let source_ids = state
        .sources
        .load()
        .iter()
        .map(|s| s.id)
        .collect::<Vec<_>>();

    let results = searcher
        .search_and_retrieve(&db, &model, &source_ids, 10, &query)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(item, _)| item)
        .collect::<Vec<_>>();

    Ok(results)
}

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            let app_state = app.state::<AppState>();

            let database = Database::new(None)?;
            let sources = perceive_core::sources::db::list_sources(&database)?;
            app_state.sources.store(Arc::new(sources));
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
                window.emit("load_status", Some(result)).ok();
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![load_status, search])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

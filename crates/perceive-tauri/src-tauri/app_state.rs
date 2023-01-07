use perceive_core::{db::Database, model::Model};

pub struct AppState {
    model: Model,
    database: Database,
}

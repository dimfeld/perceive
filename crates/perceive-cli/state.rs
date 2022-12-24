#[derive(Default)]
pub struct AppState {
    pub model: perceive_search::Model,
    pub candidates: Vec<String>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            model: perceive_search::Model::new(),
            candidates: vec![
                // "Hello world!".to_string(),
                // "Hello planet!".to_string(),
                // "Goodbye World!".to_string(),
            ],
        }
    }
}

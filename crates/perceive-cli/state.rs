#[derive(Default)]
pub struct AppState {
    pub model: perceive_core::Model,
    pub candidates: Vec<String>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            model: perceive_core::Model::new(),
            candidates: vec![
                // "Hello world!".to_string(),
                // "Hello planet!".to_string(),
                // "Goodbye World!".to_string(),
            ],
        }
    }
}

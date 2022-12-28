use std::path::PathBuf;

use perceive_core::{
    db::Database,
    model::{Model, SentenceEmbeddingsModelType},
    sources::Source,
};

pub struct AppState {
    pub model: Option<Model>,
    pub database: Database,
    pub sources: Vec<Source>,
}

impl AppState {
    pub fn new(db_path: Option<PathBuf>) -> Result<Self, eyre::Report> {
        let db = Database::new(db_path)?;

        let sources = perceive_core::sources::db::list_sources(&db)?;

        Ok(AppState {
            model: Some(Model::new_pretrained(
                SentenceEmbeddingsModelType::AllMiniLmL12V2,
            )?),
            database: db,
            sources,
        })
    }

    pub fn loan_model(&mut self) -> Model {
        self.model.take().expect("Model is not loaned twice")
    }

    pub fn return_model(&mut self, model: Model) {
        self.model = Some(model);
    }
}

use std::path::PathBuf;

use perceive_core::{
    db::Database,
    model::{Model, SentenceEmbeddingsModelType},
    sources::Source,
};

pub struct AppState {
    pub model: Option<Model>,
    pub model_id: u32,
    pub model_version: u32,
    pub highlights_model: Model,
    pub database: Database,
    pub sources: Vec<Source>,
    pub searcher: perceive_core::search::Searcher,
}

impl AppState {
    pub fn new(db_path: Option<PathBuf>) -> Result<Self, eyre::Report> {
        let db = Database::new(db_path)?;

        let model_type = SentenceEmbeddingsModelType::MsMarcoBertBaseDotV5;
        let model_id = model_type.model_id();
        let model_version = 0;

        let sources = perceive_core::sources::db::list_sources(&db)?;
        println!("Building search...");
        let searcher = perceive_core::search::Searcher::build(&db, model_id, model_version)?;

        Ok(AppState {
            model: Some(Model::new_pretrained(model_type)?),
            model_id,
            model_version,
            highlights_model: Model::new_pretrained(SentenceEmbeddingsModelType::AllMiniLmL6V2)?,
            database: db,
            searcher,
            sources,
        })
    }

    pub fn borrow_model(&self) -> &Model {
        self.model.as_ref().unwrap()
    }

    pub fn loan_model(&mut self) -> Model {
        self.model.take().expect("Model is not loaned twice")
    }

    pub fn return_model(&mut self, model: Model) {
        self.model = Some(model);
    }
}

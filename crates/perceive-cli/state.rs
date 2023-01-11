use std::{path::PathBuf, sync::Arc, time::Duration};

use eyre::eyre;
use perceive_core::{
    db::Database,
    model::{Model, SentenceEmbeddingsModelType},
    sources::Source,
};

pub struct AppState {
    pub model: Arc<Model>,
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

        let (searcher, (main_model, highlights_model)) = std::thread::scope(|scope| {
            let searcher = scope.spawn(|| {
                let start = std::time::Instant::now();

                let progress =
                    indicatif::ProgressBar::new_spinner().with_message("Rebuilding search...");
                progress.enable_steady_tick(Duration::from_millis(200));

                let searcher =
                    perceive_core::search::Searcher::build(&db, model_id, model_version)?;

                let final_msg = format!("Built search in {} seconds\n", start.elapsed().as_secs());
                progress.finish_with_message(final_msg);

                Ok::<_, eyre::Report>(searcher)
            });

            let models = scope.spawn(|| {
                let main_model = Model::new_pretrained(model_type)?;
                let highlights_model =
                    Model::new_pretrained(SentenceEmbeddingsModelType::AllMiniLmL6V2)?;
                Ok::<_, eyre::Report>((main_model, highlights_model))
            });

            Ok::<_, eyre::Report>((
                searcher.join().map_err(|e| eyre!("{e:?}"))??,
                models.join().map_err(|e| eyre!("{e:?}"))??,
            ))
        })?;

        let sources = perceive_core::sources::db::list_sources(&db)?;

        Ok(AppState {
            model: Arc::new(main_model),
            model_id,
            model_version,
            highlights_model,
            database: db,
            searcher,
            sources,
        })
    }
}

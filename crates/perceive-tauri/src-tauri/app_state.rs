use std::{ops::Deref, sync::Arc};

use arc_swap::{access::Map, ArcSwap, ArcSwapOption, Guard};
use perceive_core::{
    db::Database,
    model::{Model, SentenceEmbeddingsModelType},
    search::Searcher,
    sources::Source,
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AsyncBuilderError {
    #[error("Still loading")]
    NotLoaded,
}

pub struct AppState {
    pub model: AsyncBuilder<Model>,
    pub model_type: SentenceEmbeddingsModelType,
    pub model_id: u32,
    pub model_version: u32,
    pub sources: Vec<Source>,
    pub searcher: AsyncBuilder<Searcher>,
}

impl Default for AppState {
    fn default() -> Self {
        let model_type = SentenceEmbeddingsModelType::MsMarcoBertBaseDotV5;
        let model_id = model_type.model_id();

        Self {
            model: AsyncBuilder::new(),
            model_type,
            model_id,
            model_version: 0,
            sources: Vec::new(),
            searcher: AsyncBuilder::default(),
        }
    }
}

impl AppState {
    pub fn get_model(&self) -> Result<Arc<Model>, AsyncBuilderError> {
        self.model.0.load_full().ok_or(AsyncBuilderError::NotLoaded)
    }

    pub fn get_model_ref(&self) -> Guard<Option<Arc<Model>>> {
        self.model.0.load()
    }

    pub fn get_searcher(&self) -> Result<Arc<Searcher>, AsyncBuilderError> {
        self.searcher
            .0
            .load_full()
            .ok_or(AsyncBuilderError::NotLoaded)
    }

    pub fn get_searcher_ref(&self) -> Guard<Option<Arc<Searcher>>> {
        self.searcher.0.load()
    }

    pub fn build_searcher(
        &self,
        database: Database,
    ) -> oneshot::Receiver<Result<(), eyre::Report>> {
        let model_id = self.model_id;
        let model_version = self.model_version;
        self.searcher.rebuild(move || {
            perceive_core::search::Searcher::build(&database, model_id, model_version)
        })
    }
}

pub struct AsyncBuilder<T: Send + Sync + 'static>(Arc<ArcSwapOption<T>>);

impl<T: Send + Sync + 'static> Default for AsyncBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + Sync + 'static> AsyncBuilder<T> {
    fn do_build<E: Send + 'static>(
        f: impl FnOnce() -> Result<T, E> + Send + 'static,
        output: Arc<ArcSwapOption<T>>,
    ) -> oneshot::Receiver<Result<(), E>> {
        let (tx, rx) = oneshot::channel();
        std::thread::spawn(move || {
            let result = f();
            match result {
                Ok(value) => {
                    output.store(Some(Arc::new(value)));
                    tx.send(Ok(())).ok();
                }
                Err(e) => {
                    tx.send(Err(e)).ok();
                }
            }
        });
        rx
    }

    pub fn new() -> Self {
        Self(Arc::new(ArcSwapOption::empty()))
    }

    pub fn build<E: Send + 'static>(
        f: impl FnOnce() -> Result<T, E> + Send + 'static,
    ) -> (AsyncBuilder<T>, oneshot::Receiver<Result<(), E>>) {
        let value = AsyncBuilder::new();
        let rx = value.rebuild(f);
        (value, rx)
    }

    /// Rebuild in the background, and swap in the new value when done.
    pub fn rebuild<E: Send + 'static>(
        &self,
        f: impl FnOnce() -> Result<T, E> + Send + 'static,
    ) -> oneshot::Receiver<Result<(), E>> {
        Self::do_build(f, self.0.clone())
    }

    pub fn loaded(&self) -> bool {
        self.0.load().is_some()
    }
}

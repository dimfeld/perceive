mod sources;

use std::sync::Arc;

use rust_bert::{
    pipelines::sentence_embeddings::{
        SentenceEmbeddingsBuilder, SentenceEmbeddingsModel,
        SentenceEmbeddingsModelType as RBSentenceEmbeddingsModelType,
    },
    RustBertError,
};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumIter, EnumString, EnumVariantNames};
use tch::{Kind, Tensor};
use time::OffsetDateTime;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ItemMetadata {
    pub name: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub mtime: Option<OffsetDateTime>,
    pub atime: Option<OffsetDateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Item {
    pub source_id: i64,
    /// The path, URL, etc. of the item
    pub external_id: String,
    pub hash: Option<String>,
    pub content: Option<String>,
    #[serde(flatten)]
    pub metadata: ItemMetadata,
}

#[derive(
    Debug, Clone, Copy, Eq, PartialEq, Display, EnumString, AsRefStr, EnumIter, EnumVariantNames,
)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum SentenceEmbeddingsModelType {
    AllMiniLmL6V2,
    AllMiniLmL12V2,
    DistiluseBaseMultilingualCased,
    BertBaseNliMeanTokens,
    AllDistilrobertaV1,
    ParaphraseAlbertSmallV2,
    SentenceT5Base,
}

impl From<SentenceEmbeddingsModelType> for RBSentenceEmbeddingsModelType {
    fn from(value: SentenceEmbeddingsModelType) -> Self {
        match value {
            SentenceEmbeddingsModelType::DistiluseBaseMultilingualCased => {
                RBSentenceEmbeddingsModelType::DistiluseBaseMultilingualCased
            }
            SentenceEmbeddingsModelType::BertBaseNliMeanTokens => {
                RBSentenceEmbeddingsModelType::BertBaseNliMeanTokens
            }
            SentenceEmbeddingsModelType::AllMiniLmL12V2 => {
                RBSentenceEmbeddingsModelType::AllMiniLmL12V2
            }
            SentenceEmbeddingsModelType::AllMiniLmL6V2 => {
                RBSentenceEmbeddingsModelType::AllMiniLmL6V2
            }
            SentenceEmbeddingsModelType::AllDistilrobertaV1 => {
                RBSentenceEmbeddingsModelType::AllDistilrobertaV1
            }
            SentenceEmbeddingsModelType::ParaphraseAlbertSmallV2 => {
                RBSentenceEmbeddingsModelType::ParaphraseAlbertSmallV2
            }
            SentenceEmbeddingsModelType::SentenceT5Base => {
                RBSentenceEmbeddingsModelType::SentenceT5Base
            }
        }
    }
}

#[derive(Default, Clone)]
pub struct Model {
    model: Option<(SentenceEmbeddingsModelType, Arc<SentenceEmbeddingsModel>)>,
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model")
            .field("model_type", &self.model.as_ref().map(|m| m.0))
            .finish()
    }
}

impl Model {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn needs_load(&self, model_type: SentenceEmbeddingsModelType) -> bool {
        match &self.model {
            Some((loaded_model_type, _)) => *loaded_model_type != model_type,
            None => true,
        }
    }

    pub fn current_model(
        &self,
    ) -> Option<(SentenceEmbeddingsModelType, Arc<SentenceEmbeddingsModel>)> {
        self.model.as_ref().map(|(mt, m)| (*mt, m.clone()))
    }

    /// Return a model of a particular type, loading it if necessary
    /// If the model is the same as the previous type, it is reused.
    pub fn model(
        &mut self,
        model_type: SentenceEmbeddingsModelType,
    ) -> Result<Arc<SentenceEmbeddingsModel>, RustBertError> {
        {
            match self.model.as_ref() {
                Some((mt, m)) if mt == &model_type => return Ok(m.clone()),
                _ => {}
            };
        }

        let model = Arc::new(SentenceEmbeddingsBuilder::remote(model_type.into()).create_model()?);
        self.model = Some((model_type, model.clone()));
        Ok(model)
    }
}

pub fn dot_product(set1: &Tensor, set2: &Tensor) -> Tensor {
    set1.matmul(&set2.transpose(0, 1))
}

pub fn cosine_similarity_single_query(query: &Tensor, matches: &Tensor) -> Tensor {
    let query = query / query.linalg_norm(2.0, vec![0i64].as_slice(), true, Kind::Float);
    let matches = matches / matches.linalg_norm(2.0, vec![1i64].as_slice(), true, Kind::Float);
    dot_product(&query, &matches)
}

pub fn cosine_similarity_multi_query(set1: &Tensor, set2: &Tensor) -> Tensor {
    let set1 = set1 / set1.linalg_norm(2.0, vec![1i64].as_slice(), true, Kind::Float);
    let set2 = set2 / set2.linalg_norm(2.0, vec![1i64].as_slice(), true, Kind::Float);
    dot_product(&set1, &set2)
}

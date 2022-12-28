use rust_bert::{
    pipelines::sentence_embeddings::{
        SentenceEmbeddingsBuilder, SentenceEmbeddingsConfig, SentenceEmbeddingsModel,
        SentenceEmbeddingsModelType as RBSentenceEmbeddingsModelType,
    },
    RustBertError,
};
use strum::{AsRefStr, Display, EnumIter, EnumString, EnumVariantNames};
use tch::Tensor;

/// The supported model types. This is the same as the version built into rust_bert,
/// except for excluding some model types not recommended for these purposes.
#[derive(
    Debug,
    Clone,
    Copy,
    Hash,
    Eq,
    PartialEq,
    Display,
    EnumString,
    AsRefStr,
    EnumIter,
    EnumVariantNames,
)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum SentenceEmbeddingsModelType {
    AllMiniLmL6V2,
    AllMiniLmL12V2,
    DistiluseBaseMultilingualCased,
    AllDistilrobertaV1,
    ParaphraseAlbertSmallV2,
}

impl From<SentenceEmbeddingsModelType> for RBSentenceEmbeddingsModelType {
    fn from(value: SentenceEmbeddingsModelType) -> Self {
        match value {
            SentenceEmbeddingsModelType::DistiluseBaseMultilingualCased => {
                RBSentenceEmbeddingsModelType::DistiluseBaseMultilingualCased
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
        }
    }
}

pub struct Model {
    pub model_type: SentenceEmbeddingsModelType,
    pub model: SentenceEmbeddingsModel,
}

impl Model {
    pub fn new_pretrained(model_type: SentenceEmbeddingsModelType) -> Result<Model, RustBertError> {
        // let model_builder = SentenceEmbeddingsBuilder::remote(model_type.into());
        // let model_builder = model_builder.with_device(tch::Device::Mps);
        // let model = model_builder.create_model()?;

        let mut model_config =
            SentenceEmbeddingsConfig::from(RBSentenceEmbeddingsModelType::from(model_type));
        // #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
        // model_config.device = tch::Device::Mps;

        let model = SentenceEmbeddingsModel::new(model_config)?;

        Ok(Model { model_type, model })
    }

    pub fn encode<S: AsRef<str> + Sync>(&self, sentences: &[S]) -> Result<Tensor, RustBertError> {
        // TODO This should eventually be a more manual process so that we can do things like split
        // the input into chunks appropriate for model size, and recombine them at the end.
        let embeddings = self.model.encode_as_tensor(sentences)?;

        Ok(embeddings.embeddings)
    }
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model")
            .field("model_type", &self.model_type)
            .finish()
    }
}

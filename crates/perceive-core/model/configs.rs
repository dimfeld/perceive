use std::path::PathBuf;

use once_cell::sync::Lazy;
use rust_bert::{
    pipelines::{
        common::ModelType,
        sentence_embeddings::{
            SentenceEmbeddingsConfig, SentenceEmbeddingsModelType as RBSentenceEmbeddingsModelType,
        },
    },
    resources::{LocalResource, ResourceProvider},
};
use strum::{AsRefStr, Display, EnumIter, EnumString, EnumVariantNames};

/// The supported model types.
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
    MsMarcoDistilbertDotV5,
    MsMarcoDistilbertBaseTasB,
    MsMarcoBertBaseDotV5,
}

impl SentenceEmbeddingsModelType {
    pub(super) fn config(&self) -> SentenceEmbeddingsConfig {
        match self {
            SentenceEmbeddingsModelType::AllMiniLmL6V2 => {
                SentenceEmbeddingsConfig::from(RBSentenceEmbeddingsModelType::AllMiniLmL6V2)
            }
            SentenceEmbeddingsModelType::AllMiniLmL12V2 => {
                SentenceEmbeddingsConfig::from(RBSentenceEmbeddingsModelType::AllMiniLmL12V2)
            }
            SentenceEmbeddingsModelType::DistiluseBaseMultilingualCased => {
                SentenceEmbeddingsConfig::from(
                    RBSentenceEmbeddingsModelType::DistiluseBaseMultilingualCased,
                )
            }
            SentenceEmbeddingsModelType::AllDistilrobertaV1 => {
                SentenceEmbeddingsConfig::from(RBSentenceEmbeddingsModelType::AllDistilrobertaV1)
            }
            SentenceEmbeddingsModelType::ParaphraseAlbertSmallV2 => SentenceEmbeddingsConfig::from(
                RBSentenceEmbeddingsModelType::ParaphraseAlbertSmallV2,
            ),
            SentenceEmbeddingsModelType::MsMarcoDistilbertDotV5 => {
                msmarco_distilbert_dot_v5_config()
            }
            SentenceEmbeddingsModelType::MsMarcoBertBaseDotV5 => msmarco_bert_base_dot_v5_config(),
            SentenceEmbeddingsModelType::MsMarcoDistilbertBaseTasB => {
                msmarco_distilbert_base_tas_b_config()
            }
        }
    }

    /// Map the model to the ID in the database
    pub fn model_id(&self) -> u32 {
        match self {
            SentenceEmbeddingsModelType::AllMiniLmL6V2 => 0,
            SentenceEmbeddingsModelType::AllMiniLmL12V2 => 1,
            SentenceEmbeddingsModelType::DistiluseBaseMultilingualCased => 2,
            SentenceEmbeddingsModelType::AllDistilrobertaV1 => 3,
            SentenceEmbeddingsModelType::ParaphraseAlbertSmallV2 => 4,
            SentenceEmbeddingsModelType::MsMarcoDistilbertDotV5 => 5,
            SentenceEmbeddingsModelType::MsMarcoDistilbertBaseTasB => 6,
            SentenceEmbeddingsModelType::MsMarcoBertBaseDotV5 => 7,
        }
    }
}

// TODO get this from a runtime value indicating where the app was installed
static MODEL_DATA_DIR: Lazy<String> =
    Lazy::new(|| format!("{}model_data", env!("CARGO_WORKSPACE_DIR")));

fn local_resource(model: &str, file: &str) -> Box<dyn ResourceProvider + Send> {
    let dir = Lazy::force(&MODEL_DATA_DIR);
    Box::new(LocalResource::from(PathBuf::from(format!(
        "{dir}/{model}/{file}",
    ))))
}

fn model_config(
    model_name: &str,
    transformer_type: ModelType,
    has_dense: bool,
    has_merges: bool,
) -> SentenceEmbeddingsConfig {
    let lr = |file: &str| local_resource(model_name, file);

    SentenceEmbeddingsConfig {
        modules_config_resource: lr("modules.json"),
        transformer_type,
        transformer_config_resource: lr("config.json"),
        transformer_weights_resource: lr("rust_model.ot"),
        pooling_config_resource: lr("1_Pooling/config.json"),
        dense_config_resource: has_dense.then(|| lr("2_Dense/config.json")),
        dense_weights_resource: has_dense.then(|| lr("2_Dense/rust_model.ot")),
        sentence_bert_config_resource: lr("sentence_bert_config.json"),
        tokenizer_config_resource: lr("tokenizer_config.json"),
        tokenizer_vocab_resource: lr("vocab.txt"),
        tokenizer_merges_resource: has_merges.then(|| lr("merges.txt")),
        device: tch::Device::cuda_if_available(),
    }
}

fn msmarco_bert_base_dot_v5_config() -> SentenceEmbeddingsConfig {
    model_config("msmarco-bert-base-dot-v5", ModelType::Bert, false, false)
}

fn msmarco_distilbert_dot_v5_config() -> SentenceEmbeddingsConfig {
    model_config(
        "msmarco-distilbert-dot-v5",
        ModelType::DistilBert,
        false,
        false,
    )
}

fn msmarco_distilbert_base_tas_b_config() -> SentenceEmbeddingsConfig {
    model_config(
        "msmarco-distilbert-base-tas-b",
        ModelType::DistilBert,
        false,
        false,
    )
}

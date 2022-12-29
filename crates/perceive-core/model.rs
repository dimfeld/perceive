// This file is heavily based on
// https://github.com/guillaume-be/rust-bert/blob/master/src/pipelines/sentence_embeddings/pipeline.rs,
// with some modifications to work with longer text passages.

use std::path::PathBuf;

use once_cell::sync::Lazy;
use rust_bert::{
    pipelines::{
        common::{ConfigOption, ModelType, TokenizerOption},
        sentence_embeddings::{
            layers::{Dense, DenseConfig, Pooling, PoolingConfig},
            SentenceEmbeddingsConfig, SentenceEmbeddingsModelType as RBSentenceEmbeddingsModelType,
            SentenceEmbeddingsModulesConfig, SentenceEmbeddingsOption,
            SentenceEmbeddingsSentenceBertConfig, SentenceEmbeddingsTokenizerConfig,
            SentenceEmbeddingsTokenizerOuput,
        },
    },
    resources::{LocalResource, ResourceProvider},
    Config, RustBertError,
};
use rust_tokenizers::tokenizer::TruncationStrategy;
use strum::{AsRefStr, Display, EnumIter, EnumString, EnumVariantNames};
use tch::{nn, Tensor};

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
    MsMarcoDistilbertBaseV4,
    MsMarcoDistilbertBaseTasB,
}

impl SentenceEmbeddingsModelType {
    fn config(&self) -> SentenceEmbeddingsConfig {
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
            SentenceEmbeddingsModelType::MsMarcoDistilbertBaseV4 => {
                msmarco_distilbert_base_v4_config()
            }
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
            SentenceEmbeddingsModelType::MsMarcoDistilbertBaseV4 => 5,
            SentenceEmbeddingsModelType::MsMarcoDistilbertBaseTasB => 6,
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

fn model_config(model_name: &str, has_dense: bool, has_merges: bool) -> SentenceEmbeddingsConfig {
    let lr = |file: &str| local_resource(model_name, file);

    SentenceEmbeddingsConfig {
        modules_config_resource: lr("modules.json"),
        transformer_type: ModelType::DistilBert,
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

fn msmarco_distilbert_base_v4_config() -> SentenceEmbeddingsConfig {
    model_config("msmarco-distilbert-base-v4", false, false)
}

fn msmarco_distilbert_base_tas_b_config() -> SentenceEmbeddingsConfig {
    model_config("msmarco-distilbert-base-tas-b", false, false)
}

pub struct Model {
    pub model_type: SentenceEmbeddingsModelType,

    sentence_bert_config: SentenceEmbeddingsSentenceBertConfig,
    tokenizer: TokenizerOption,
    tokenizer_truncation_strategy: TruncationStrategy,
    var_store: nn::VarStore,
    transformer: SentenceEmbeddingsOption,
    pooling_layer: Pooling,
    dense_layer: Option<Dense>,
    normalize_embeddings: bool,
}

impl Model {
    pub fn new_pretrained(model_type: SentenceEmbeddingsModelType) -> Result<Model, RustBertError> {
        let SentenceEmbeddingsConfig {
            modules_config_resource,
            sentence_bert_config_resource,
            tokenizer_config_resource,
            tokenizer_vocab_resource,
            tokenizer_merges_resource,
            transformer_type,
            transformer_config_resource,
            transformer_weights_resource,
            pooling_config_resource,
            dense_config_resource,
            dense_weights_resource,
            device,
        } = model_type.config();

        let modules =
            SentenceEmbeddingsModulesConfig::from_file(modules_config_resource.get_local_path()?)
                .validate()?;

        // Setup tokenizer

        let tokenizer_config = SentenceEmbeddingsTokenizerConfig::from_file(
            tokenizer_config_resource.get_local_path()?,
        );
        let sentence_bert_config = SentenceEmbeddingsSentenceBertConfig::from_file(
            sentence_bert_config_resource.get_local_path()?,
        );
        let tokenizer = TokenizerOption::from_file(
            transformer_type,
            tokenizer_vocab_resource
                .get_local_path()?
                .to_string_lossy()
                .as_ref(),
            tokenizer_merges_resource
                .as_ref()
                .map(|resource| resource.get_local_path())
                .transpose()?
                .map(|path| path.to_string_lossy().into_owned())
                .as_deref(),
            tokenizer_config
                .do_lower_case
                .unwrap_or(sentence_bert_config.do_lower_case),
            tokenizer_config.strip_accents,
            tokenizer_config.add_prefix_space,
        )?;

        // Setup transformer

        let mut var_store = nn::VarStore::new(tch::Device::cuda_if_available());
        let transformer_config = ConfigOption::from_file(
            transformer_type,
            transformer_config_resource.get_local_path()?,
        );
        let transformer =
            SentenceEmbeddingsOption::new(transformer_type, var_store.root(), &transformer_config)?;
        var_store.load(transformer_weights_resource.get_local_path()?)?;

        // If on M1, switch to the GPU.
        // This needs to be done here instead of during the initial load, since the pretrained
        // models can't be loaded directly.
        #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
        var_store.set_device(tch::Device::Mps);

        // Setup pooling layer

        let pooling_config = PoolingConfig::from_file(pooling_config_resource.get_local_path()?);
        let pooling_layer = Pooling::new(pooling_config);

        // Setup dense layer

        let dense_layer = if modules.dense_module().is_some() {
            let dense_config =
                DenseConfig::from_file(dense_config_resource.unwrap().get_local_path()?);
            Some(Dense::new(
                dense_config,
                dense_weights_resource.unwrap().get_local_path()?,
                device,
            )?)
        } else {
            None
        };

        let normalize_embeddings = modules.has_normalization();

        Ok(Model {
            model_type,
            sentence_bert_config,
            tokenizer,
            tokenizer_truncation_strategy: TruncationStrategy::LongestFirst,
            var_store,
            transformer,
            pooling_layer,
            dense_layer,
            normalize_embeddings,
        })
    }

    /// Tokenizes the inputs
    fn tokenize<S>(&self, inputs: &[S]) -> SentenceEmbeddingsTokenizerOuput
    where
        S: AsRef<str> + Sync,
    {
        let tokenized_input = self.tokenizer.encode_list(
            inputs,
            self.sentence_bert_config.max_seq_length,
            &self.tokenizer_truncation_strategy,
            0,
        );

        let max_len = tokenized_input
            .iter()
            .map(|input| input.token_ids.len())
            .max()
            .unwrap_or(0);

        let pad_token_id = self.tokenizer.get_pad_id().unwrap_or(0);
        let tokens_ids = tokenized_input
            .into_iter()
            .map(|input| {
                let mut token_ids = input.token_ids;
                token_ids.extend(vec![pad_token_id; max_len - token_ids.len()]);
                token_ids
            })
            .collect::<Vec<_>>();

        let tokens_masks = tokens_ids
            .iter()
            .map(|input| {
                Tensor::of_slice(
                    &input
                        .iter()
                        .map(|&e| i64::from(e != pad_token_id))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();

        let tokens_ids = tokens_ids
            .into_iter()
            .map(|input| Tensor::of_slice(&(input)))
            .collect::<Vec<_>>();

        SentenceEmbeddingsTokenizerOuput {
            tokens_ids,
            tokens_masks,
        }
    }

    pub fn encode<S: AsRef<str> + Sync>(&self, inputs: &[S]) -> Result<Tensor, RustBertError> {
        let SentenceEmbeddingsTokenizerOuput {
            tokens_ids,
            tokens_masks,
        } = self.tokenize(inputs);
        let tokens_ids = Tensor::stack(&tokens_ids, 0).to(self.var_store.device());
        let tokens_masks = Tensor::stack(&tokens_masks, 0).to(self.var_store.device());

        let (tokens_embeddings, _all_attentions) =
            tch::no_grad(|| self.transformer.forward(&tokens_ids, &tokens_masks))?;

        let mean_pool =
            tch::no_grad(|| self.pooling_layer.forward(tokens_embeddings, &tokens_masks));
        let maybe_linear = if let Some(dense_layer) = &self.dense_layer {
            tch::no_grad(|| dense_layer.forward(&mean_pool))
        } else {
            mean_pool
        };
        let maybe_normalized = if self.normalize_embeddings {
            let norm = &maybe_linear
                .norm_scalaropt_dim(2, &[1], true)
                .clamp_min(1e-12)
                .expand_as(&maybe_linear);
            maybe_linear / norm
        } else {
            maybe_linear
        };

        Ok(maybe_normalized)
    }
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model")
            .field("model_type", &self.model_type)
            .finish()
    }
}

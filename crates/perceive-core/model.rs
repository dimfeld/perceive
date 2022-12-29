// This file is heavily based on
// https://github.com/guillaume-be/rust-bert/blob/master/src/pipelines/sentence_embeddings/pipeline.rs,
// with some modifications to work with longer text passages.

mod configs;
mod highlight;
pub mod tokenize;

pub use configs::SentenceEmbeddingsModelType;
use rust_bert::{
    pipelines::{
        common::{ConfigOption, TokenizerOption},
        sentence_embeddings::{
            layers::{Dense, DenseConfig, Pooling, PoolingConfig},
            SentenceEmbeddingsConfig, SentenceEmbeddingsModulesConfig, SentenceEmbeddingsOption,
            SentenceEmbeddingsSentenceBertConfig, SentenceEmbeddingsTokenizerConfig,
            SentenceEmbeddingsTokenizerOuput,
        },
    },
    Config, RustBertError,
};
use rust_tokenizers::tokenizer::TruncationStrategy;
use tch::{nn, Tensor};

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

    pub fn encode_tokens(
        &self,
        tokens: &SentenceEmbeddingsTokenizerOuput,
    ) -> Result<Tensor, RustBertError> {
        let tokens_ids = Tensor::stack(&tokens.tokens_ids, 0).to(self.var_store.device());
        let tokens_masks = Tensor::stack(&tokens.tokens_masks, 0).to(self.var_store.device());

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

    pub fn encode<S: AsRef<str> + Sync>(&self, inputs: &[S]) -> Result<Tensor, RustBertError> {
        let tokens = self.tokenize(inputs);
        self.encode_tokens(&tokens)
    }
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model")
            .field("model_type", &self.model_type)
            .finish()
    }
}

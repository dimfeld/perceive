use std::panic::catch_unwind;

use eyre::eyre;
use rust_bert::{
    pipelines::sentence_embeddings::{
        layers::{Dense, Pooling},
        SentenceEmbeddingsOption, SentenceEmbeddingsTokenizerOuput,
    },
    RustBertError,
};
use tch::{nn, Tensor};

use super::ModelError;

pub(super) enum ModelWorkerCommand {
    Encode(ModelWorkerCommandData<SentenceEmbeddingsTokenizerOuput, tch::Tensor>),
}

pub(super) struct ModelWorkerCommandData<INPUT, OUTPUT>
where
    INPUT: Send,
    OUTPUT: Send,
{
    data: INPUT,
    return_value: oneshot::Sender<Result<OUTPUT, ModelError>>,
}

impl<INPUT: Send, OUTPUT: Send> ModelWorkerCommandData<INPUT, OUTPUT> {
    pub fn build(
        input: INPUT,
    ) -> (
        ModelWorkerCommandData<INPUT, OUTPUT>,
        oneshot::Receiver<Result<OUTPUT, ModelError>>,
    ) {
        let (chan_tx, chan_rx) = oneshot::channel();

        (
            ModelWorkerCommandData {
                data: input,
                return_value: chan_tx,
            },
            chan_rx,
        )
    }
}

pub(super) struct WorkerData {
    pub var_store: nn::VarStore,
    pub transformer: SentenceEmbeddingsOption,
    pub pooling_layer: Pooling,
    pub dense_layer: Option<Dense>,
    pub normalize_embeddings: bool,
}

pub(super) fn sentence_embeddings_model_worker(
    rx: flume::Receiver<ModelWorkerCommand>,
    model: WorkerData,
) {
    while let Ok(msg) = rx.recv() {
        match msg {
            ModelWorkerCommand::Encode(msg) => model.handle_encode_msg(msg),
        };
    }
}

impl WorkerData {
    fn handle_encode_msg(
        &self,
        msg: ModelWorkerCommandData<SentenceEmbeddingsTokenizerOuput, tch::Tensor>,
    ) {
        let result = match catch_unwind(|| self.encode_tokens(msg.data)) {
            Ok(result) => result.map_err(|e| e.into()),
            Err(e) => Err(ModelError::ModelPanic(eyre!("{:?}", e))),
        };
        msg.return_value.send(result).ok();
    }

    fn encode_tokens(
        &self,
        tokens: SentenceEmbeddingsTokenizerOuput,
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
}

use std::borrow::Cow;

use rust_bert::pipelines::sentence_embeddings::SentenceEmbeddingsTokenizerOuput;
use tch::Tensor;

use super::Model;

impl Model {
    pub(super) fn generate_token_tensors<T: AsRef<[i64]>>(
        &self,
        token_ids: &[T],
    ) -> SentenceEmbeddingsTokenizerOuput {
        let max_len = token_ids
            .iter()
            .map(|input| input.as_ref().len())
            .max()
            .unwrap_or(0);

        let pad_token_id = self.tokenizer.get_pad_id().unwrap_or(0);

        // Pad any vectors shorter than the max length so that they are all the same.
        let tokens_ids = token_ids
            .iter()
            .map(|input| {
                let input = input.as_ref();
                if input.len() == max_len {
                    Cow::Borrowed(input)
                } else {
                    let mut padded = vec![pad_token_id; max_len];
                    padded[..input.len()].copy_from_slice(input.as_ref());
                    Cow::Owned(padded)
                }
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

    /// Tokenizes the inputs
    pub fn tokenize<S>(&self, inputs: &[S]) -> SentenceEmbeddingsTokenizerOuput
    where
        S: AsRef<str> + Sync,
    {
        let tokenized_input = self
            .tokenizer
            .encode_list(
                inputs,
                self.sentence_bert_config.max_seq_length,
                &self.tokenizer_truncation_strategy,
                0,
            )
            .into_iter()
            .map(|input| input.token_ids)
            .collect::<Vec<_>>();

        self.generate_token_tensors(&tokenized_input)
    }
}

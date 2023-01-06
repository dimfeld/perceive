use itertools::Itertools;
use once_cell::sync::Lazy;

use super::{Model, ModelError};
use crate::dot_product;

static CHUNK_SIZES: Lazy<(usize, usize)> = Lazy::new(|| {
    (
        std::env::var("CHUNK_SIZE")
            .unwrap_or_default()
            .parse::<usize>()
            .unwrap_or(20),
        std::env::var("CHUNK_OVERLAP")
            .unwrap_or_default()
            .parse::<usize>()
            .unwrap_or(4),
    )
});

impl Model {
    /// Given a query and a set of matching documents, try to find the
    /// chunk of text from each document that best matches to the query.
    pub fn highlight<'s, 'doc, S: AsRef<str> + Sync>(
        &'s self,
        query: &'doc str,
        documents: &'doc [S],
    ) -> Result<Vec<Option<&'doc str>>, ModelError> {
        // Encode the query
        let query_encoding = self.encode(&[query])?;

        // Tokenize all the documents, with no length limit on each one
        let tokenized_docs = self.tokenizer.encode_list(
            documents,
            // Don't truncate here since we're chunking below.
            1_000_000,
            &rust_tokenizers::tokenizer::TruncationStrategy::LongestFirst,
            0,
        );

        // Split the document into chunks, with a bit of ovelap on each side.
        let &(chunk_size, chunk_overlap) = Lazy::force(&CHUNK_SIZES);
        // i.e 0..10, 8..18, 16..26
        let chunk_index_inc = chunk_size - chunk_overlap;

        let total_chunks = tokenized_docs
            .iter()
            .map(|t| t.token_ids.len() / chunk_index_inc + 1)
            .sum::<usize>();
        let mut token_chunks = Vec::with_capacity(total_chunks);
        let mut token_chunk_boundaries = Vec::with_capacity(total_chunks);
        let mut document_chunk_boundaries = Vec::with_capacity(tokenized_docs.len());

        for tokens in &tokenized_docs {
            // Make sure we add a chunk on short documents
            let mut i = 0;
            while i + chunk_overlap < tokens.token_ids.len() {
                let start_index = i;
                let end_index = std::cmp::min(i + chunk_size, tokens.token_ids.len());
                let special_tokens_mask = &tokens.special_tokens_mask[start_index..end_index];

                // Find the longest consecutive sequence of non-special tokens
                let mut longest_start = 0;
                let mut longest_length = 0;
                let mut current_start = 0;
                let mut current_length = 0;

                for (index, &is_special) in special_tokens_mask.iter().enumerate() {
                    if is_special == 0 {
                        current_length += 1;
                    } else {
                        if current_length > longest_length {
                            longest_start = current_start;
                            longest_length = current_length;
                        }

                        current_start = index;
                        current_length = 0;
                    }
                }

                if current_length > longest_length {
                    longest_start = current_start;
                    longest_length = current_length;
                }

                let start_index = start_index + longest_start;
                let end_index = std::cmp::min(start_index + longest_length, end_index);

                if end_index - start_index >= chunk_size / 2 {
                    let chunk = &tokens.token_ids[start_index..end_index];

                    token_chunk_boundaries.push(start_index..end_index);
                    token_chunks.push(chunk);
                }

                i += chunk_index_inc;
            }

            document_chunk_boundaries.push(token_chunks.len());
        }

        let tensors = self.generate_token_tensors(&token_chunks);

        // Get the scores for all the chunks across all the documents.
        let scores = if tensors.tokens_ids.is_empty() {
            Vec::new()
        } else {
            let docs_encoding = self.encode_tokens(tensors)?;
            let scores: Vec<f32> = dot_product(&query_encoding, &docs_encoding).into();
            scores
        };

        // Now we need to find the best chunk for each document.
        let mut highlights = Vec::with_capacity(documents.len());
        for (index, &overall_chunk_end) in document_chunk_boundaries.iter().enumerate() {
            let overall_chunk_start = if index == 0 {
                0
            } else {
                document_chunk_boundaries[index - 1]
            };
            let doc_scores = &scores[overall_chunk_start..overall_chunk_end];
            let Some(best_chunk_index) = doc_scores
                .iter()
                .position_max_by(|a, b| a.partial_cmp(b).unwrap()) else {
                    highlights.push(None);
                    continue;
            };

            let doc_best_chunk_range =
                token_chunk_boundaries[overall_chunk_start + best_chunk_index].clone();

            // Now we have the indexes of the tokens where the best chunk
            // starts and ends, relative to the document itself, so we can
            // go back to the original tokenized input to figure out where
            // in the document these tokens occur.
            let (text_start, text_end) = tokenized_docs[index].token_offsets[doc_best_chunk_range]
                .iter()
                .filter_map(|o| o.as_ref())
                .fold((0, 0), |(begin, end), offset| {
                    if begin == 0 && end == 0 {
                        (offset.begin, offset.end)
                    } else {
                        (
                            std::cmp::min(begin, offset.begin),
                            std::cmp::max(end, offset.end),
                        )
                    }
                });

            let doc = documents[index].as_ref();
            let mut doc_iter = doc.char_indices();
            let start_bytes = doc_iter.nth(text_start as usize).map(|c| c.0);
            let end_bytes = doc_iter.nth((text_end - text_start) as usize).map(|c| c.0);

            let highlight = start_bytes
                .zip(end_bytes)
                .map(|(start, end)| &doc[start..end])
                .unwrap_or_default();

            highlights.push(Some(highlight));
        }

        Ok(highlights)
    }
}

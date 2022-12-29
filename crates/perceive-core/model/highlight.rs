use itertools::Itertools;
use once_cell::sync::Lazy;
use rust_bert::RustBertError;

use super::Model;
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
    ) -> Result<Vec<&'doc str>, RustBertError> {
        // Encode the query
        let query_encoding = self.encode(&[query])?;

        // Tokenize all the documents, with no length limit on each one
        let tokenized = self.tokenizer.encode_list(
            documents,
            // Don't truncate here since we're chunking below.
            1_000_000_000,
            &rust_tokenizers::tokenizer::TruncationStrategy::LongestFirst,
            0,
        );

        // Split the document into chunks, with a bit of ovelap on each side.
        let &(chunk_size, chunk_overlap) = Lazy::force(&CHUNK_SIZES);
        // i.e 0..10, 8..18, 16..26
        let chunk_index_inc = chunk_size - chunk_overlap;

        let total_chunks = tokenized
            .iter()
            .map(|t| t.token_ids.len() / chunk_index_inc + 1)
            .sum::<usize>();
        let mut token_chunks = Vec::with_capacity(total_chunks);
        let mut token_chunk_ends = Vec::with_capacity(tokenized.len());

        for tokens in tokenized.iter() {
            // Make sure we add a chunk on short documents
            let first = &tokens.token_ids[0..chunk_size.min(tokens.token_ids.len())];
            token_chunks.push(first);

            let mut i = chunk_index_inc;
            while i + chunk_overlap < tokens.token_ids.len() {
                let start_index = i;
                let end_index = std::cmp::min(i + chunk_size, tokens.token_ids.len());

                let chunk = &tokens.token_ids[start_index..end_index];
                token_chunks.push(chunk);
                i += chunk_index_inc;
            }

            token_chunk_ends.push(token_chunks.len());
        }

        let tensors = self.generate_token_tensors(&token_chunks);

        // Get the scores for all the chunks across all the documents.
        let docs_encoding = self.encode_tokens(&tensors)?;
        let scores: Vec<f32> = dot_product(&query_encoding, &docs_encoding).into();

        // Now we need to find the best chunk for each document.
        let mut highlights = Vec::with_capacity(documents.len());
        for (index, &overall_chunk_end) in token_chunk_ends.iter().enumerate() {
            let overall_chunk_start = if index == 0 {
                0
            } else {
                token_chunk_ends[index - 1]
            };
            let doc_scores = &scores[overall_chunk_start..overall_chunk_end];
            let best_chunk_index = doc_scores
                .iter()
                .position_max_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap();

            // From the chunk index, calculate which tokens this corresponded to
            let doc_best_chunk_start = best_chunk_index * chunk_index_inc;

            // Now we have the indexes of the tokens where the best chunk
            // starts and ends, relative to the document itself, so we can
            // go back to the original tokenized input to figure out where
            // in the document these tokens occur.
            let (text_start, text_end) = tokenized[index]
                .token_offsets
                .iter()
                .skip(doc_best_chunk_start)
                .take(chunk_size)
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

            highlights.push(highlight);
        }

        Ok(highlights)
    }
}

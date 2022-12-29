use super::Model;
use crate::Item;

impl Model {
    pub fn highlight<S: AsRef<str> + Sync>(&self, query: &str, documents: &[S]) {
        // Encode the query
        let query_encoding = self.encode(&[query]);

        // For each document, tokenize, chunk, and encode each chunk. Score each chunk against the
        // query and return the offsets of the top-scoring chunks.

        let tokenized = self.tokenizer.encode_list(
            documents,
            // Don't truncate here since we're chunking below.
            1_000_000_000,
            &rust_tokenizers::tokenizer::TruncationStrategy::LongestFirst,
            0,
        );

        let chunk_size = 20;
        // overlap by this amount on both sides
        let chunk_overlap = 5;
        let chunk_index_inc = chunk_size - (chunk_overlap * 2);

        let total_chunks = tokenized
            .iter()
            .map(|t| t.token_ids.len() / chunk_index_inc + 1)
            .sum::<usize>();
        let mut token_chunks = Vec::with_capacity(total_chunks);
        let mut token_chunk_ends = Vec::with_capacity(tokenized.len());

        for (index, tokens) in tokenized.iter().enumerate() {
            // Make sure we add a chunk on short documents
            let first = &tokens.token_ids[0..chunk_size.min(tokens.token_ids.len())];

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
    }
}

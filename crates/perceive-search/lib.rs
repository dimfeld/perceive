use rust_bert::{
    pipelines::sentence_embeddings::{
        SentenceEmbeddingsBuilder, SentenceEmbeddingsModelOuput, SentenceEmbeddingsModelType,
    },
    RustBertError,
};
use tch::{Kind, Tensor};

pub fn embed(s: &[impl AsRef<str> + Sync]) -> Result<SentenceEmbeddingsModelOuput, RustBertError> {
    // Set-up sentence embeddings model
    let model = SentenceEmbeddingsBuilder::remote(SentenceEmbeddingsModelType::AllMiniLmL12V2)
        .create_model()?;

    model.encode_as_tensor(s)
}

pub fn cosine_similarity(set1: &Tensor, set2: &Tensor) -> Tensor {
    let set1 = set1 / set1.linalg_norm(2.0, vec![1i64].as_slice(), true, Kind::Float);
    let set2 = set2 / set2.linalg_norm(2.0, vec![1i64].as_slice(), true, Kind::Float);
    set1.matmul(&set2.transpose(0, 1))
}

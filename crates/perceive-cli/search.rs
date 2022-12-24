use eyre::Result;
use perceive_search as psearch;

pub fn search() -> Result<()> {
    let sentences = vec!["Hello world!", "Hello planet!", "Goodbye World!"];
    let results = psearch::embed(&sentences)?;

    let cos_score = psearch::cosine_similarity(&results.embeddings, &results.embeddings);

    let scores: Vec<Vec<f32>> = Vec::from(cos_score);

    for i in 0..sentences.len() {
        for j in 0..sentences.len() {
            println!(
                "{first:<20} {second:<20}   {score:.2}",
                first = sentences[i],
                second = sentences[j],
                score = scores[i][j]
            );
        }
    }

    Ok(())
}

use perceive_search as search;

fn main() {
    let sentences = &["Hello world!", "Hello planet!", "Goodbye World!"];
    let results = search::embed(sentences).unwrap();

    let cos_score = search::cosine_similarity(&results.embeddings, &results.embeddings);

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
}

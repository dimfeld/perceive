use std::rc::Rc;

use ahash::HashSet;
#[cfg(feature = "cli")]
use indicatif::ProgressStyle;
use rayon::prelude::*;
use rusqlite::Connection;
use time::OffsetDateTime;

use crate::{
    db::{Database, DbError},
    model::Model,
    Item, ItemMetadata,
};

// This ensures that blas_src actually gets included, since it's not used elsewhere.
// https://github.com/rust-ndarray/ndarray#how-to-enable-blas-integration
extern crate blas_src;

#[derive(Debug, Copy, Clone)]
pub struct SearchItem {
    pub id: i64,
    pub score: f32,
}

struct SourceSearch {
    id: i64,
    hnsw: instant_distance::HnswMap<Point, i64>,
}

pub struct Searcher {
    sources: Vec<SourceSearch>,
    /// The search structure is built only from non-hidden items, but this stores IDs of items
    /// that were hidden after the search was built, to avoid needing to rebuild it after every single
    /// hide operation.
    pub hidden: HashSet<i64>,
}

impl Searcher {
    pub fn build(
        database: &Database,
        model_id: u32,
        model_version: u32,
        #[cfg(feature = "cli")] progress: Option<indicatif::MultiProgress>,
    ) -> Result<Searcher, eyre::Report> {
        let conn = database.read_pool.get()?;

        let mut sources_stmt = conn.prepare("SELECT id, name FROM sources")?;
        let sources = sources_stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let sources = Self::build_sources(
            &conn,
            model_id,
            model_version,
            sources,
            #[cfg(feature = "cli")]
            progress,
        )?;

        Ok(Searcher {
            sources,
            hidden: HashSet::default(),
        })
    }

    pub fn rebuild_source(
        &mut self,
        database: &Database,
        source_id: i64,
        source_name: String,
        model_id: u32,
        model_version: u32,
        #[cfg(feature = "cli")] progress: Option<indicatif::MultiProgress>,
    ) -> Result<(), eyre::Report> {
        let conn = database.read_pool.get()?;

        let sources = Self::build_sources(
            &conn,
            model_id,
            model_version,
            vec![(source_id, source_name)],
            #[cfg(feature = "cli")]
            progress,
        )?;

        let Some(result_source) = sources.into_iter().next() else {
            return Ok(());
        };

        match self.sources.iter().position(|s| s.id == source_id) {
            Some(index) => self.sources[index] = result_source,
            None => self.sources.push(result_source),
        }

        Ok(())
    }

    fn build_sources(
        conn: &Connection,
        model_id: u32,
        model_version: u32,
        sources: Vec<(i64, String)>,
        #[cfg(feature = "cli")] progress: Option<indicatif::MultiProgress>,
    ) -> Result<Vec<SourceSearch>, eyre::Report> {
        let mut sources = sources
            .into_iter()
            .map(|(id, name)| (id, name, Vec::new(), Vec::new()))
            .collect::<Vec<_>>();

        let mut stmt = conn.prepare(
            r##"SELECT items.id, source_id, embedding
        FROM items
        JOIN item_embeddings ie ON model_id=? AND model_version=? AND ie.item_id=items.id
        WHERE skipped IS NULL AND hidden_at IS NULL"##,
        )?;

        let rows = stmt.query_and_then([model_id, model_version], |row| {
            let value: (i64, i64, Vec<f32>) = (
                row.get(0)?,
                row.get(1)?,
                deserialize_embedding(row.get_ref(2)?.as_blob().map_err(DbError::query)?),
            );

            Ok::<_, DbError>(value)
        })?;

        for row in rows {
            let (id, source, embedding) = row?;
            let source_points = sources.iter_mut().find(|x| x.0 == source);
            let Some(source_points) = source_points else {
                continue;
            };

            source_points.2.push(Point::from(embedding));
            source_points.3.push(id);
        }

        let longest_name = sources.iter().map(|x| x.1.len()).max().unwrap_or(20);
        #[cfg(feature = "cli")]
        let progress_template = format!("{{prefix:{longest_name}}} {{bar:40}} {{pos}}/{{len}}");

        let source_search = sources
            .into_par_iter()
            .filter(|(_, _, points, _)| !points.is_empty())
            .map(|(source_id, name, points, values)| {
                let hnsw = instant_distance::Builder::default();

                #[cfg(feature = "cli")]
                let hnsw = if let Some(progress) = &progress {
                    let bar = indicatif::ProgressBar::new(0)
                        .with_prefix(name)
                        .with_style(ProgressStyle::with_template(&progress_template).unwrap());

                    hnsw.progress(progress.add(bar))
                } else {
                    hnsw
                };

                let hnsw = hnsw.build(points, values);

                SourceSearch {
                    id: source_id,
                    hnsw,
                }
            })
            .collect::<Vec<_>>();

        Ok(source_search)
    }

    pub fn search_vector(
        &self,
        sources: &[i64],
        num_results: usize,
        vector: Vec<f32>,
    ) -> Vec<SearchItem> {
        let search_point = Point::from(vector);

        let mut results = self
            .sources
            .par_iter()
            .filter(|source| sources.contains(&source.id))
            .flat_map_iter(|source| {
                let mut searcher = instant_distance::Search::default();
                source
                    .hnsw
                    .search(&search_point, &mut searcher)
                    .filter(|item| !self.hidden.contains(item.value))
                    .take(num_results)
                    .map(|item| SearchItem {
                        id: *item.value,
                        score: item.distance,
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        results.sort_unstable_by(|a, b| a.score.partial_cmp(&b.score).unwrap());
        results.truncate(num_results);
        results
    }

    pub fn search(
        &self,
        model: &Model,
        sources: &[i64],
        num_results: usize,
        query: &str,
    ) -> Vec<SearchItem> {
        let term_embedding = encode_query(model, query);
        self.search_vector(sources, num_results, term_embedding)
    }

    pub fn search_vector_and_retrieve(
        &self,
        database: &Database,
        sources: &[i64],
        num_results: usize,
        vector: Vec<f32>,
    ) -> Result<Vec<(Item, SearchItem)>, DbError> {
        let items = self.search_vector(sources, num_results, vector);

        let values = items
            .iter()
            .map(|item| rusqlite::types::Value::from(item.id))
            .collect::<Vec<_>>();

        let conn = database.read_pool.get()?;
        let mut stmt = conn.prepare_cached(
            r##"SELECT id, source_id, external_id, content, name, author, description, modified, last_accessed
            FROM items WHERE skipped is NULL AND hidden_at IS NULL AND id IN rarray(?)"##)?;

        let mut rows = stmt
            .query_map([Rc::new(values)], |row| {
                Ok(Item {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    external_id: row.get(2)?,
                    content: row.get(3)?,
                    raw_content: None,
                    hash: None,
                    skipped: None,
                    process_version: 0,
                    metadata: ItemMetadata {
                        name: row.get(4)?,
                        author: row.get(5)?,
                        description: row.get(6)?,
                        mtime: row
                            .get::<_, Option<i64>>(7)?
                            .map(|t| OffsetDateTime::from_unix_timestamp(t).unwrap()),
                        atime: row
                            .get::<_, Option<i64>>(8)?
                            .map(|t| OffsetDateTime::from_unix_timestamp(t).unwrap()),
                    },
                })
            })?
            .map(|row| {
                let item = row?;
                let result = items.iter().find(|i| i.id == item.id).copied().unwrap();
                Ok::<_, DbError>((item, result))
            })
            .collect::<Result<Vec<_>, DbError>>()?;

        rows.sort_unstable_by(|a, b| a.1.score.partial_cmp(&b.1.score).unwrap());
        Ok(rows)
    }

    pub fn search_and_retrieve(
        &self,
        database: &Database,
        model: &Model,
        sources: &[i64],
        num_results: usize,
        query: &str,
    ) -> Result<Vec<(Item, SearchItem)>, DbError> {
        let vector = encode_query(model, query);
        self.search_vector_and_retrieve(database, sources, num_results, vector)
    }
}

pub fn encode_query(model: &Model, query: &str) -> Vec<f32> {
    Vec::from(model.encode(&[query]).unwrap()).pop().unwrap()
}

#[derive(Clone)]
pub struct Point(ndarray::Array1<f32>);

impl From<tch::Tensor> for Point {
    fn from(value: tch::Tensor) -> Self {
        let v = Vec::<f32>::from(value);
        Point(ndarray::Array1::from(v))
    }
}

impl From<Vec<f32>> for Point {
    fn from(value: Vec<f32>) -> Self {
        Point(ndarray::Array1::from(value))
    }
}

impl instant_distance::Point for Point {
    fn distance(&self, other: &Self) -> f32 {
        0.0 - self.0.dot(&other.0)
    }
}

pub fn deserialize_embedding(value: &[u8]) -> Vec<f32> {
    value
        .chunks(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

pub fn serialize_embedding(embedding: &[f32]) -> Vec<u8> {
    let mut bytes_vec = Vec::with_capacity(embedding.len() * std::mem::size_of::<f32>());
    for value in embedding {
        bytes_vec.extend(value.to_le_bytes());
    }
    bytes_vec
}

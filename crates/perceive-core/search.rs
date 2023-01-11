use std::rc::Rc;

use ahash::HashSet;
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
    hnsw: hnsw_rs::hnsw::Hnsw<f32, NdArrayDistance>,
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
    ) -> Result<Searcher, eyre::Report> {
        let conn = database.read_pool.get()?;

        let mut sources_stmt = conn.prepare("SELECT id FROM sources")?;
        let sources = sources_stmt
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        let sources = Self::build_sources(&conn, model_id, model_version, &sources)?;

        Ok(Searcher {
            sources,
            hidden: HashSet::default(),
        })
    }

    pub fn rebuild_source(
        &mut self,
        database: &Database,
        source_id: i64,
        model_id: u32,
        model_version: u32,
    ) -> Result<(), eyre::Report> {
        let conn = database.read_pool.get()?;

        let sources = Self::build_sources(&conn, model_id, model_version, &[source_id])?;

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
        sources: &[i64],
    ) -> Result<Vec<SourceSearch>, eyre::Report> {
        let mut stmt = conn.prepare(
            r##"SELECT items.id, source_id, embedding
        FROM items
        JOIN item_embeddings ie ON model_id=? AND model_version=? AND ie.item_id=items.id
        WHERE skipped IS NULL AND hidden_at IS NULL"##,
        )?;

        let rows = stmt
            .query_and_then([model_id, model_version], |row| {
                let value: (usize, i64, Vec<f32>) = (
                    row.get(0)?,
                    row.get(1)?,
                    deserialize_embedding(row.get_ref(2)?.as_blob().map_err(DbError::query)?),
                );

                Ok::<_, DbError>(value)
            })?
            .map(|row| {
                let row = row?;

                Ok(sources
                    .iter()
                    .position(|&s| s == row.1)
                    .map(|source_idx| (row.0, source_idx, row.2)))
            })
            .filter_map(|r| r.transpose())
            .collect::<Result<Vec<_>, eyre::Report>>()?;

        let items_per_source = rows
            .par_iter()
            .fold(
                || vec![0; sources.len()],
                |mut items_per_source, (_, source_idx, _)| {
                    items_per_source[*source_idx] += 1;
                    items_per_source
                },
            )
            .reduce(
                || vec![0; sources.len()],
                |mut a, b| {
                    for (a, b) in a.iter_mut().zip(b.iter()) {
                        *a += b;
                    }
                    a
                },
            );

        let mut sources = sources
            .iter()
            .zip(items_per_source.into_iter())
            .map(|(&id, num_elements)| {
                let num_layers = 16.min((num_elements as f32).ln().trunc() as usize);
                let hnsw =
                    hnsw_rs::hnsw::Hnsw::new(64, num_elements, num_layers, 800, NdArrayDistance {});

                SourceSearch { id, hnsw }
            })
            .collect::<Vec<_>>();

        rows.into_par_iter().for_each(|(id, source_idx, point)| {
            sources[source_idx].hnsw.insert_slice((&point, id));
        });

        for source in sources.iter_mut() {
            source.hnsw.set_searching_mode(true);
        }

        Ok(sources)
    }

    pub fn search_vector(
        &self,
        sources: &[i64],
        num_results: usize,
        vector: Vec<f32>,
    ) -> Vec<SearchItem> {
        let mut results = self
            .sources
            .par_iter()
            .filter(|source| sources.contains(&source.id))
            .flat_map_iter(|source| {
                source
                    .hnsw
                    .search(&vector, num_results, 24)
                    .into_iter()
                    .map(|n| SearchItem {
                        id: n.d_id as i64,
                        score: n.distance,
                    })
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
pub struct NdArrayDistance {}

impl hnsw_rs::dist::Distance<f32> for NdArrayDistance {
    fn eval(&self, va: &[f32], vb: &[f32]) -> f32 {
        let a = ndarray::ArrayView1::from(va);
        let b = ndarray::ArrayView1::from(vb);

        let dot = a.dot(&b);
        let result = 1.0 - (dot / (va.len() as f32));

        result.max(0.0)
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

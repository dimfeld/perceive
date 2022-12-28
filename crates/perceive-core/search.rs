use std::rc::Rc;

use ahash::HashMap;
use instant_distance::MapItem;
use itertools::Itertools;

use crate::{
    db::{Database, DbError},
    Item, ItemMetadata,
};

pub struct Search {
    hnsw: instant_distance::HnswMap<Point, i64>,
}

impl Search {
    fn build(
        database: &Database,
        model_id: u32,
        model_version: u32,
    ) -> Result<Search, eyre::Report> {
        let conn = database.read_pool.get()?;

        let mut stmt = conn.prepare_cached(
            r##"SELECT id, embedding
        FROM items
        JOIN item_embeddings ie ON model_id=? AND model_version=? AND ie.item_id=items.id"##,
        )?;

        let rows = stmt.query_and_then([model_id, model_version], |row| {
            let value: (i64, Vec<f32>) = (
                row.get(0)?,
                deserialize_embedding(row.get_ref(1)?.as_blob().map_err(DbError::query)?),
            );

            Ok::<_, DbError>(value)
        })?;

        let mut points = Vec::new();
        let mut values = Vec::new();

        for row in rows {
            let (id, embedding) = row?;
            points.push(Point(embedding));
            values.push(id);
        }

        let hnsw = instant_distance::Builder::default().build(points, values);

        Ok(Search { hnsw })
    }

    fn search(&self, term_embedding: &Point) -> Vec<MapItem<Point, i64>> {
        let mut searcher = instant_distance::Search::default();
        let results = self
            .hnsw
            .search(term_embedding, &mut searcher)
            .collect::<Vec<_>>();

        results
    }

    fn search_and_retrieve(
        &self,
        database: &Database,
        term_embedding: &Point,
    ) -> Result<Vec<(Item, &MapItem<Point, i64>)>, DbError> {
        let ids = self.search(term_embedding);
        let conn = database.read_pool.get()?;

        let values = ids
            .iter()
            .map(|item| rusqlite::types::Value::from(*item.value))
            .collect::<Vec<_>>();

        let mut stmt = conn.prepare_cached(
            r##"SELECT id, source_id, external_id, content, name, author, description, modified, last_accessed
            FROM items WHERE id IN rarray(?)"##)?;

        let rows = stmt
            .query_map([Rc::new(values)], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    Item {
                        source_id: row.get(1)?,
                        external_id: row.get(2)?,
                        content: row.get(3)?,
                        hash: None,
                        metadata: ItemMetadata {
                            name: row.get(4)?,
                            author: row.get(5)?,
                            description: row.get(6)?,
                            mtime: row.get(7)?,
                            atime: row.get(8)?,
                        },
                    },
                ))
            })?
            .map(|row| {
                let (id, item) = row?;
                let result = ids.iter().find(|i| *i.value == id).unwrap();
                Ok::<_, DbError>((item, result))
            })
            .collect::<Result<Vec<_>, DbError>>();

        rows
    }
}

#[derive(Clone)]
pub struct Point(Vec<f32>);

impl instant_distance::Point for Point {
    fn distance(&self, other: &Self) -> f32 {
        1.0 - self
            .0
            .iter()
            .zip(other.0.iter())
            .map(|(a, b)| (a * b))
            .sum::<f32>()
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

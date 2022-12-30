pub mod batch_sender;
pub mod db;
pub mod model;
pub mod paths;
pub mod search;
pub mod sources;
pub mod time_tracker;

use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use tch::{Kind, Tensor};
use time::OffsetDateTime;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ItemMetadata {
    pub name: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub mtime: Option<OffsetDateTime>,
    pub atime: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Copy, Serialize, Display, EnumString, Deserialize)]
#[strum(serialize_all = "snake_case")]
pub enum SkipReason {
    NotFound,
    FetchError,
    Unauthorized,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Item {
    pub source_id: i64,
    /// The path, URL, etc. of the item
    pub external_id: String,
    pub hash: Option<String>,
    pub content: Option<String>,
    pub raw_content: Option<Vec<u8>>,
    pub process_version: i32,
    pub metadata: ItemMetadata,
    pub skipped: Option<SkipReason>,
}

pub fn dot_product(set1: &Tensor, set2: &Tensor) -> Tensor {
    set1.matmul(&set2.transpose(0, 1))
}

pub fn cosine_similarity_single_query(query: &Tensor, matches: &Tensor) -> Tensor {
    let query = query / query.linalg_norm(2.0, [0i64].as_slice(), true, Kind::Float);
    let matches = matches / matches.linalg_norm(2.0, [1i64].as_slice(), true, Kind::Float);
    dot_product(&query, &matches)
}

pub fn cosine_similarity_multi_query(set1: &Tensor, set2: &Tensor) -> Tensor {
    let set1 = set1 / set1.linalg_norm(2.0, [1i64].as_slice(), true, Kind::Float);
    let set2 = set2 / set2.linalg_norm(2.0, [1i64].as_slice(), true, Kind::Float);
    dot_product(&set1, &set2)
}

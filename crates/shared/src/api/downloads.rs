use serde::{Deserialize, Serialize};

/// Body of `POST /files/zip/selections`: entries of the folder `path` to zip, each a single
/// name. The answer's id then names the zip: `GET /files/zip?selection=<id>`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ZipSelection {
    pub volume: String,
    pub path: String,
    pub names: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ZipSelectionCreated {
    pub id: String,
}

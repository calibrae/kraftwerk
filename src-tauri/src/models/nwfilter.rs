//! Network filter (nwfilter) summary surfaced to the frontend.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NwFilterInfo {
    pub name: String,
    pub uuid: String,
}

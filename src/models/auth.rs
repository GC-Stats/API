/*
    GC-Stats — API

    API key model and the SHA-256 hashing helper used to store and look up
    keys without persisting them in plaintext.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use serde::{Serialize, Deserialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;

#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: u64,
    pub client_name: String,
    pub rate_limit: i32,
    pub is_active: bool,
}

pub fn hash_api_key(raw: &str) -> String {
    format!("{:x}", Sha256::digest(raw.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_api_key_is_hex_sha256() {
        assert_eq!(
            hash_api_key("test"),
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
    }
}
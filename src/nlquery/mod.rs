/*
    GC-Stats — API

    Declares the nlquery module: the natural-language-to-Cube-query pipeline
    backing `/internal/nl-query` (config, error type, Cube query schema and
    validation, system prompt construction, provider resolution, and the
    Cube.dev client).

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

pub mod config;
pub mod cube;
pub mod error;
pub mod prompt;
pub mod provider;
pub mod query;

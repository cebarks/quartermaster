// Error types are incrementally used by CLI commands (tasks 7-12).
// Some variants are not yet used but will be in subsequent tasks.
#![allow(dead_code)]

use thiserror::Error;

#[derive(Debug, Error)]
pub enum QumaError {
    #[error("SPT directory not found — run `quma setup` or pass --spt-dir")]
    SptDirNotFound,

    #[error("not a valid SPT 4.0+ install: {0}")]
    InvalidSptDir(String),

    #[error("config file not found: {0}")]
    ConfigNotFound(std::path::PathBuf),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("forge API error: {0}")]
    ForgeApi(String),

    #[error("forge API request failed: {0}")]
    ForgeHttp(#[from] reqwest::Error),

    #[error("archive error: {0}")]
    Archive(String),

    #[error("mod not found: {0}")]
    ModNotFound(String),

    #[error("mod conflict: file {path} already belongs to mod {owner}")]
    FileConflict { path: String, owner: String },

    #[error("server is running — queue the operation or use --force")]
    ServerRunning,
}

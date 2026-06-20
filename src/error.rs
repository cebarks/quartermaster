use thiserror::Error;

#[derive(Debug, Error)]
pub enum QumaError {
    #[error("SPT directory not found — run `quma setup` or pass --spt-dir")]
    SptDirNotFound,

    #[error("not a valid SPT 4.0+ install: {0}")]
    InvalidSptDir(String),
}

//! The only module that writes to the database. Every mutation appends one event; `events` is
//! never updated or deleted. Faces (`hooks`, `cli`) call in here and translate errors.

pub mod events;
pub mod pending_calls;
pub mod sessions;
pub mod tasks;

use std::fmt;

// Consumed by hooks::dispatch and cli::session_cmd (Tasks 10, 11) once a face translates
// service errors into an exit code or a printed message.
#[allow(dead_code)]
#[derive(Debug)]
pub enum ServiceError {
    Db(rusqlite::Error),
    NotFound(String),
    #[allow(dead_code)] // Consumed by services::sessions and services::tasks (Tasks 7, 8).
    Invalid(String),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceError::Db(e) => write!(f, "database: {e}"),
            ServiceError::NotFound(m) => write!(f, "{m}"),
            ServiceError::Invalid(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<rusqlite::Error> for ServiceError {
    fn from(e: rusqlite::Error) -> Self {
        ServiceError::Db(e)
    }
}

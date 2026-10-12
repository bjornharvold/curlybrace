//! Fixtures and an in-memory `DocumentStore`, compiled for tests and when the
//! `test-support` feature is enabled.

mod bson_order;
mod memory_store;
pub mod mock_objects;
mod query_eval;

pub use memory_store::InMemoryStore;

pub mod client;
pub mod clock;
pub mod compare;
pub mod connection;
mod connection_color;
pub mod connection_folders;
pub mod connection_names;
pub mod connection_repair;
pub mod copy;
pub mod crud;
pub mod driver_error;
pub mod ejson;
pub mod error;
pub mod history;
pub mod indexes;
mod mongo_indexes;
pub mod mongo_store;
mod mongo_write;
mod mongo_write_fields;
pub mod query;
pub mod read_only_store;
#[cfg(test)]
mod read_only_store_tests;
pub mod runtime;
pub mod secrets;
pub mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod tunnel;
pub mod uri;
mod uri_host;
#[cfg(test)]
mod uri_host_joiner_tests;
#[cfg(test)]
mod uri_host_tests;
pub mod uri_problem;
#[cfg(test)]
mod uri_problem_tests;

/// True when this build can authenticate with Kerberos (GSSAPI). The UI hides
/// the Kerberos option when false.
pub const GSSAPI_AVAILABLE: bool = cfg!(feature = "gssapi");

#[cfg(test)]
mod tests {
    #[test]
    fn gssapi_available_matches_feature() {
        assert_eq!(super::GSSAPI_AVAILABLE, cfg!(feature = "gssapi"));
    }
}

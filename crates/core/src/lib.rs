pub mod client;
pub mod connection;
pub mod ejson;
pub mod error;
pub mod query;
pub mod secrets;
pub mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

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

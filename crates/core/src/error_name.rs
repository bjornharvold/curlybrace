//! The name of a [`CoreError`] variant, for logs that must never carry the error's payload (a
//! `Database` message can quote a query or a document).

use crate::error::CoreError;

/// The variant name of `error`. The match is exhaustive on purpose: a new variant does not
/// compile until it is named here.
pub(crate) fn variant_of(error: &CoreError) -> &'static str {
    match error {
        CoreError::InvalidUri(..) => "InvalidUri",
        CoreError::InvalidName => "InvalidName",
        CoreError::InvalidFolderName => "InvalidFolderName",
        CoreError::InvalidJson(..) => "InvalidJson",
        CoreError::InvalidDocument(..) => "InvalidDocument",
        CoreError::Secret(..) => "Secret",
        CoreError::Ssh(..) => "Ssh",
        CoreError::Unsupported(..) => "Unsupported",
        CoreError::Database(..) => "Database",
        CoreError::NotFound => "NotFound",
        CoreError::Superseded => "Superseded",
        CoreError::SshHostKeyChanged => "SshHostKeyChanged",
        CoreError::AuthFailed => "AuthFailed",
        CoreError::Unauthorized => "Unauthorized",
        CoreError::Unreachable => "Unreachable",
        CoreError::TlsHandshake => "TlsHandshake",
        CoreError::TlsHandshakeSsh => "TlsHandshakeSsh",
        CoreError::ConnectionInterrupted(..) => "ConnectionInterrupted",
        CoreError::UnreachableSsh => "UnreachableSsh",
        CoreError::UnreachableWrite => "UnreachableWrite",
        CoreError::UnreachableWriteSsh => "UnreachableWriteSsh",
        CoreError::WriteOutcomeUnknown => "WriteOutcomeUnknown",
        CoreError::TlsThroughTunnel => "TlsThroughTunnel",
        CoreError::CompareDuplicateKey => "CompareDuplicateKey",
        CoreError::CompareKeyMissing => "CompareKeyMissing",
        CoreError::CompareKeyInvalid => "CompareKeyInvalid",
        CoreError::CompareKeyOrder => "CompareKeyOrder",
        CoreError::CompareKeyNotIndexed => "CompareKeyNotIndexed",
        CoreError::CompareCapExceeded => "CompareCapExceeded",
        CoreError::CompareSetupInvalid => "CompareSetupInvalid",
        CoreError::ApplyTargetNotWritable => "ApplyTargetNotWritable",
        CoreError::ApplyWriteRejected => "ApplyWriteRejected",
        CoreError::ApplyDuplicateKey => "ApplyDuplicateKey",
        CoreError::ApplyOutcomeUnknown => "ApplyOutcomeUnknown",
        CoreError::ExportWriteFailed => "ExportWriteFailed",
        CoreError::ExportFileNotRemoved => "ExportFileNotRemoved",
        CoreError::ReadOnly => "ReadOnly",
        CoreError::CopyRefused(..) => "CopyRefused",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_variant_names_without_payload() {
        let secret = "zebra-secret";
        let samples = [
            (CoreError::Database(secret.into()), "Database"),
            (CoreError::Unsupported(secret.into()), "Unsupported"),
            (
                CoreError::ConnectionInterrupted(secret.into()),
                "ConnectionInterrupted",
            ),
            (CoreError::InvalidJson(secret.into()), "InvalidJson"),
            (CoreError::Unauthorized, "Unauthorized"),
            (CoreError::Unreachable, "Unreachable"),
            (CoreError::ReadOnly, "ReadOnly"),
        ];
        for (error, name) in samples {
            assert_eq!(variant_of(&error), name);
            assert!(!variant_of(&error).contains(secret));
        }
    }
}

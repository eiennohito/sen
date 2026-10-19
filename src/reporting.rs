//! Privacy-preserving error sink. Only typed codes and programmer-owned Bug labels are printed.
use std::{
    error::Error,
    time::{SystemTime, UNIX_EPOCH},
};

/// Log the full chain's categories, preserving OS/SQLite codes without payloads.
/// RPC errors may embed tokens or message bodies, so their Display is not safe.
pub fn log(operation: &str, kind: &str, error: &(dyn Error + 'static)) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    eprintln!(
        "{}.{:03} operation={} kind={} chain={}",
        stamp.as_secs(),
        stamp.subsec_millis(),
        operation,
        kind,
        chain(error)
    );
}

fn chain(error: &(dyn Error + 'static)) -> String {
    let mut chain = Vec::new();
    let mut current = Some(error);
    while let Some(e) = current {
        chain.push(if let Some(e) = e.downcast_ref::<std::io::Error>() {
            format!("io:{:?}:os={:?}", e.kind(), e.raw_os_error())
        } else if let Some(e) = e.downcast_ref::<rusqlite::Error>() {
            match e {
                rusqlite::Error::SqliteFailure(code, _) => {
                    format!("sqlite:{:?}:extended={}", code.code, code.extended_code)
                }
                _ => "sqlite:operation".into(),
            }
        } else if let Some(category) = crate::client::error::diagnostic(e) {
            category
        } else if let Some(e) = e.downcast_ref::<crate::store::StoreError>() {
            match e {
                crate::store::StoreError::Sql(cause) => {
                    current = Some(cause);
                    continue;
                }
                crate::store::StoreError::Io(cause) => {
                    current = Some(cause);
                    continue;
                }
                crate::store::StoreError::Json(_) => "store:json".into(),
                crate::store::StoreError::Invalid(_) => "store-invariant".into(),
            }
        } else {
            "context(redacted)".into()
        });
        current = e.source();
    }
    chain.join(" -> ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transparent_store_errors_and_sign_in_keep_typed_causes() {
        let io = crate::store::StoreError::Io(std::io::Error::from_raw_os_error(libc::ENOSPC));
        let output = chain(&io);
        assert!(output.contains("StorageFull"));
        assert!(output.contains(&format!("os=Some({})", libc::ENOSPC)));
        assert!(!output.contains("invariant"));
        let sql = crate::store::StoreError::Sql(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE),
            Some("private-message".into()),
        ));
        let output = chain(&sql);
        assert!(output.contains(&format!(
            "extended={}",
            rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
        )));
        assert!(!output.contains("invariant"));
        assert!(!output.contains("private-message"));
        let sign_in = crate::client::error::SignInFailure::CryptoModule("private-path".into());
        assert_eq!(chain(&sign_in), "sign-in:crypto-module");
        assert_eq!(
            chain(&crate::client::error::Failure::Bug("decode".into())),
            "client-bug:decode"
        );
    }
    #[test]
    fn diagnostic_chains_keep_categories_without_private_payloads() {
        let error = anyhow::Error::new(std::io::Error::from_raw_os_error(libc::ENOSPC))
            .context("token=private-token message=private-message u1234567890123456");
        let output = chain(error.as_ref());
        assert!(output.contains("StorageFull"));
        for secret in ["private-token", "private-message", "u1234567890123456"] {
            assert!(!output.contains(secret));
        }
        let invalid = crate::store::StoreError::Invalid("window anchor not in chat".into());
        assert!(chain(&invalid).contains("store-invariant"));
        let untrusted = crate::store::StoreError::Invalid("private-message".into());
        assert!(!chain(&untrusted).contains("private-message"));
    }
}

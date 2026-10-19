//! Single privacy-preserving error sink. Untrusted diagnostic text is never printed.
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
            format!("sqlite:{:?}", e.sqlite_error_code())
        } else if let Some(e) = e.downcast_ref::<crate::client::RpcError>() {
            format!("rpc:http={}:code={}", e.http, e.code)
        } else if let Some(e) = e.downcast_ref::<crate::client::error::Failure>() {
            match e {
                crate::client::error::Failure::Bug(message) => format!("bug:{message}"),
                _ => e.to_string(),
            }
        } else if let Some(e) = e.downcast_ref::<crate::client::error::Error>() {
            match e {
                crate::client::error::Error::SignIn(
                    reason @ crate::client::error::SignInFailure::CryptoChanged { .. },
                ) => format!("{reason:?}"),
                crate::client::error::Error::SignIn(reason) => format!("sign-in:{}", reason.kind()),
            }
        } else if let Some(e) = e.downcast_ref::<crate::store::StoreError>() {
            match e {
                crate::store::StoreError::Invalid(message) => safe_invariant(message).into(),
                crate::store::StoreError::Json(_) => "stored-json-decode".into(),
                crate::store::StoreError::NewerSchema(version) => {
                    format!("unsupported-schema:{version}")
                }
                crate::store::StoreError::Sql(e) => format!("sqlite:{:?}", e.sqlite_error_code()),
                crate::store::StoreError::Io(e) => {
                    format!("io:{:?}:os={:?}", e.kind(), e.raw_os_error())
                }
            }
        } else {
            "context(redacted)".into()
        });
        current = e.source();
    }
    chain.join(" -> ")
}

// Only programmer-owned literals are safe. A future diagnostic that embeds a
// request, token, or message body must not accidentally bypass redaction.
fn safe_invariant(message: &str) -> &str {
    match message {
        "window anchor not in chat"
        | "sync worker could not start"
        | "request sequence exhausted"
        | "send sequence missing"
        | "read marker missing"
        | "row already confirmed with another id"
        | "server id belongs to another chat"
        | "message chat mismatch"
        | "request sequence belongs to another chat"
        | "unknown send request"
        | "send request mismatch"
        | "expected failure delivery"
        | "confirmed row cannot fail or retry"
        | "only failed rows can retry"
        | "only failed rows can be discarded"
        | "noncanonical server id" => message,
        _ => "invalid-store-operation(details redacted)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(chain(&invalid).contains("window anchor not in chat"));
        let untrusted = crate::store::StoreError::Invalid("private-message".into());
        assert!(!chain(&untrusted).contains("private-message"));
    }
}

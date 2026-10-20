//! Privacy-preserving error sink. Only typed codes and programmer-owned Bug labels are printed.
use std::{
    error::Error,
    time::{SystemTime, UNIX_EPOCH},
};

thread_local! {
    static OPERATION: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}
pub(crate) fn set_operation(operation: &str) {
    OPERATION.with(|value| *value.borrow_mut() = Some(operation.into()));
}
pub(crate) fn source(error: &(dyn Error + 'static)) {
    OPERATION.with(|value| {
        log(
            value.borrow().as_deref().unwrap_or("network"),
            "source",
            error,
        )
    });
}

/// Log the full chain's categories, preserving OS/SQLite codes without payloads.
/// RPC errors may embed tokens or message bodies, so their Display is not safe.
pub fn log(operation: &str, kind: &str, error: &(dyn Error + 'static)) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    eprintln!(
        "{}.{:03} {}",
        stamp.as_secs(),
        stamp.subsec_millis(),
        format_event(operation, kind, error)
    );
}

/// Report each panic once, before the owning thread's recovery boundary unwinds.
pub(crate) fn install_panic_hook() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let payload = info.payload();
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic payload");
            let thread = std::thread::current();
            OPERATION.with(|value| {
                let operation = value.borrow();
                let operation = operation
                    .as_deref()
                    .unwrap_or_else(|| thread.name().unwrap_or("engine"));
                let location = info
                    .location()
                    .map(|location| {
                        format!(
                            "{}:{}:{}",
                            location.file(),
                            location.line(),
                            location.column()
                        )
                    })
                    .unwrap_or_else(|| "unknown".into());
                eprintln!("operation={operation} kind=panic location={location} payload={message}");
            });
            previous(info);
        }))
    });
}

fn format_event(operation: &str, kind: &str, error: &(dyn Error + 'static)) -> String {
    format!("operation={operation} kind={kind} chain={}", chain(error))
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
    fn panic_reports_keep_default_diagnostics_and_operation() {
        const CHILD: &str = "LINE_TEST_REPORTING_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "reporting::tests::panic_reports_keep_default_diagnostics_and_operation",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("RUST_BACKTRACE", "1")
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{stderr}");
            for operation in ["probe-regression", "fallback-thread"] {
                assert!(
                    stderr.contains(&format!(
                        "operation={operation} kind=panic location=src/reporting.rs:"
                    )),
                    "{stderr}"
                );
            }
            assert!(stderr.contains("panicked at src/reporting.rs:"), "{stderr}");
            assert!(
                stderr.contains("assertion `left == right` failed"),
                "{stderr}"
            );
            assert!(stderr.contains("stack backtrace:"), "{stderr}");
            assert_eq!(stderr.matches("payload=assertion").count(), 2, "{stderr}");
            return;
        }
        install_panic_hook();
        for named_operation in [true, false] {
            assert!(
                std::thread::Builder::new()
                    .name("fallback-thread".into())
                    .spawn(move || {
                        if named_operation {
                            set_operation("probe-regression");
                        }
                        assert_eq!(1, 2, "panic-hook-regression");
                    })
                    .unwrap()
                    .join()
                    .is_err()
            );
        }
    }
    #[test]
    fn rpc_log_includes_operation_method_gateway_and_codes() {
        let error = crate::client::error::RpcError {
            method: "/api/talk/thrift/Talk/TalkService/getRecentMessagesV2".into(),
            http: 400,
            gateway: 10051,
            code: 0,
            status: 0,
        };
        assert_eq!(
            format_event("refresh-recent", "source", &error),
            "operation=refresh-recent kind=source chain=rpc:getRecentMessagesV2:http=400:gateway=10051:code=0"
        );
    }
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

//! The single network-policy boundary.
use super::http::TransportError;
use std::fmt;
pub(super) const TALK_EXCEPTION: i64 = 10051;
pub(super) const UNKNOWN_GATEWAY: i64 = 99999;

#[derive(Debug)]
pub(crate) struct RpcError {
    pub(crate) gateway: i64,
    pub(crate) method: String,
    pub(crate) http: u16,
    pub(crate) code: i64,
    pub(crate) status: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Failure {
    #[error("request not written")]
    NotDelivered,
    #[error("request unanswered")]
    Unanswered,
    #[error("signed out: {0}")]
    SignedOut(SignOutReason),
    #[error("request refused: {0}")]
    Refused(Refusal),
    /// Programmer-owned diagnostics only; never interpolate identifiers or untrusted text.
    #[error("internal failure: {0}")]
    Bug(String),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SignOutReason {
    #[error("no session")]
    NoSession,
    #[error("sign-in expired")]
    AuthenticationFailed,
    #[error("this account is unavailable")]
    NotAvailableUser,
    #[error("this device is no longer authorized")]
    NotAuthorizedDevice,
    #[error("the access token could not be renewed")]
    TokenRefresh,
    #[error("the session belongs to another account")]
    IdentityChanged,
    #[error("{0}")]
    Unavailable(SignInFailure),
}

/// Whether hyper may have delivered the request, with a redacted transport cause.
#[derive(Debug, thiserror::Error)]
#[error("HTTPS transport failed; response not logged")]
pub(crate) struct TransportFailure {
    pub write_started: bool,
    pub source: TransportError,
}

/// Stable rejection categories; diagnostic errors never become row text.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
pub enum Refusal {
    #[error("internal error")]
    Internal,
    #[error(transparent)]
    Server(#[from] ServerRefusal),
    #[error("signed out")]
    SignedOut,
    #[error("no answer received")]
    Unanswered,
    #[error("app closed before sending")]
    AppClosed,
    #[error("File missing")]
    FileMissing,
    #[serde(other)]
    #[serde(rename = "Message was not accepted")]
    #[error("rejection: {self:?}")]
    NotAccepted,
}

/// Rejections returned by the remote services.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
pub enum ServerRefusal {
    #[error("HTTP {0}")]
    Http(u16),
    #[error("gateway code {0}")]
    Gateway(i64),
    #[error("TalkException code {0}")]
    Talk(i64),
}

/// Prerequisites and protocol outcomes carried intact to the sign-in surface.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("sign-in: {}", self.kind())]
pub enum SignInFailure {
    CryptoModule(std::path::PathBuf),
    ExtensionMissing,
    CryptoChanged { version: String, missing: String },
    StateDirectory(std::path::PathBuf),
    Keyring,
    Network,
    Server,
    Phone,
    Cancelled,
    SavedSession,
    SessionStoreMismatch,
    SessionKeyMissing,
    SessionIo,
    Expired,
    SessionIdentity,
    Internal,
}

#[derive(Debug, thiserror::Error)]
#[error("HTTPS request cancelled")]
pub(crate) struct Cancelled;

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: HTTP {}, LINE code {}, status {}",
            self.method, self.http, self.code, self.status
        )
    }
}
impl std::error::Error for RpcError {}

pub(super) fn find_source<T: std::error::Error + Send + Sync + 'static>(
    error: &anyhow::Error,
) -> Option<&T> {
    error
        .downcast_ref::<T>()
        .or_else(|| causes(error).find_map(|cause| cause.downcast_ref::<T>()))
}

fn causes(error: &anyhow::Error) -> impl Iterator<Item = &(dyn std::error::Error + 'static)> {
    std::iter::successors(
        Some(error.as_ref() as &(dyn std::error::Error + 'static)),
        |cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .and_then(std::io::Error::get_ref)
                .map(|inner| inner as &(dyn std::error::Error + 'static))
                .or_else(|| cause.source())
        },
    )
}

pub fn classify(error: &anyhow::Error) -> Failure {
    if let Some(reason) = error.downcast_ref::<SignInFailure>() {
        return match reason {
            SignInFailure::Network => Failure::NotDelivered,
            _ => Failure::SignedOut(SignOutReason::Unavailable(reason.clone())),
        };
    }
    if let Some(failure) = error.downcast_ref::<Failure>() {
        return failure.clone();
    }
    for cause in causes(error) {
        if let Some(error) = cause.downcast_ref::<TransportFailure>() {
            return match error.source {
                TransportError::Io(_)
                | TransportError::Timeout
                | TransportError::ConnectionFailed
                | TransportError::ProxyConfiguration => {
                    if error.write_started {
                        Failure::Unanswered
                    } else {
                        Failure::NotDelivered
                    }
                }
                _ => Failure::Bug("HTTP protocol failure".into()),
            };
        }
        if let Some(error) = cause.downcast_ref::<RpcError>() {
            if error.gateway == UNKNOWN_GATEWAY {
                return Failure::Refused(Refusal::Server(ServerRefusal::Gateway(UNKNOWN_GATEWAY)));
            }
            return match error.code {
                1 => Failure::SignedOut(SignOutReason::AuthenticationFailed),
                7 => Failure::SignedOut(SignOutReason::NotAvailableUser),
                8 => Failure::SignedOut(SignOutReason::NotAuthorizedDevice),
                code if code != 0 || error.gateway == TALK_EXCEPTION => {
                    Failure::Refused(Refusal::Server(ServerRefusal::Talk(code)))
                }
                _ if error.http != 200 => {
                    Failure::Refused(Refusal::Server(ServerRefusal::Http(error.http)))
                }
                _ => Failure::Refused(Refusal::Server(ServerRefusal::Gateway(
                    if error.gateway == 0 {
                        UNKNOWN_GATEWAY
                    } else {
                        error.gateway
                    },
                ))),
            };
        }
        if let Some(error) = cause.downcast_ref::<super::desktop::assets::Failure>() {
            return match error {
                super::desktop::assets::Failure::Network
                | super::desktop::assets::Failure::Timeout => Failure::Unanswered,
                super::desktop::assets::Failure::Http(code) => {
                    Failure::Refused(Refusal::Server(ServerRefusal::Http(*code)))
                }
                _ => Failure::Bug("media response failure".into()),
            };
        }
    }
    Failure::Bug("decode, crypto, protocol or invariant failure".into())
}

impl SignInFailure {
    pub fn is_prerequisite(&self) -> bool {
        matches!(
            self,
            Self::CryptoModule(_) | Self::ExtensionMissing | Self::CryptoChanged { .. }
        )
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Self::CryptoModule(_) | Self::ExtensionMissing | Self::CryptoChanged { .. } => {
                "crypto-module"
            }
            Self::StateDirectory(_) => "state-directory",
            Self::Keyring => "keyring",
            Self::Network => "network",
            Self::Server => "server",
            Self::Phone => "phone",
            Self::Cancelled => "cancelled",
            Self::SavedSession => "saved-session-corrupt",
            Self::SessionStoreMismatch => "saved-session-store-mismatch",
            Self::SessionKeyMissing => "saved-session-key-missing",
            Self::SessionIo => "saved-session-io",
            Self::Internal => "internal",
            Self::Expired => "expired",
            Self::SessionIdentity => "session-identity",
        }
    }
}

pub(super) fn needs_refresh<T>(result: &anyhow::Result<T>) -> bool {
    result
        .as_ref()
        .err()
        .and_then(|error| error.downcast_ref::<RpcError>())
        .is_some_and(RpcError::needs_refresh)
}

pub fn sign_in_failure(error: &anyhow::Error, cancelled: bool) -> SignInFailure {
    if cancelled {
        return SignInFailure::Cancelled;
    }
    match classify(error) {
        Failure::SignedOut(SignOutReason::Unavailable(reason)) => reason,
        Failure::NotDelivered | Failure::Unanswered => SignInFailure::Network,
        Failure::Refused(_) => SignInFailure::Server,
        Failure::SignedOut(_) => SignInFailure::Expired,
        Failure::Bug(_) => SignInFailure::Internal,
    }
}

impl wasmi::core::HostError for SignInFailure {}

impl Failure {
    /// Invalid chat requests should not be repeated automatically by resync.
    pub fn suppress_resync_refresh(&self) -> bool {
        matches!(
            self,
            Self::Refused(Refusal::Server(
                ServerRefusal::Http(400) | ServerRefusal::Talk(0)
            ))
        )
    }

    pub fn key_rotated(&self) -> bool {
        matches!(
            self,
            Self::Refused(Refusal::Server(ServerRefusal::Talk(82 | 84)))
        )
    }
    pub fn retry_send(&self) -> bool {
        matches!(
            self,
            Self::NotDelivered
                | Self::Refused(Refusal::Server(
                    ServerRefusal::Talk(115) | ServerRefusal::Gateway(UNKNOWN_GATEWAY)
                ))
        )
    }

    pub fn retry_read(&self) -> bool {
        matches!(
            self,
            Self::NotDelivered
                | Self::Unanswered
                | Self::Refused(Refusal::Server(
                    ServerRefusal::Talk(115)
                        | ServerRefusal::Gateway(UNKNOWN_GATEWAY)
                        | ServerRefusal::Http(408 | 429 | 500..=599)
                ))
        )
    }
}

pub(crate) fn media_failure(error: &anyhow::Error) -> super::desktop::assets::Failure {
    use super::desktop::assets::Failure as Media;
    if let Some(failure) = error.downcast_ref::<Media>() {
        return *failure;
    }
    if error.downcast_ref::<Cancelled>().is_some() {
        return Media::Cancelled;
    }
    if error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return Media::Http(404);
    }
    if let Some(transport) = find_source::<TransportFailure>(error) {
        return match transport.source {
            TransportError::Timeout => Media::Timeout,
            TransportError::Io(_)
            | TransportError::ConnectionFailed
            | TransportError::ProxyConfiguration => Media::Network,
            _ => Media::Decode,
        };
    }
    if let Some(rpc) = find_source::<RpcError>(error) {
        return Media::Http(rpc.http);
    }
    Media::Decode
}

impl super::desktop::assets::Failure {
    pub fn retry_read(self) -> bool {
        self.retry_automatically() || matches!(self, Self::Cancelled | Self::Http(403 | 408 | 429))
    }
    pub fn retry_automatically(self) -> bool {
        matches!(self, Self::Network | Self::Timeout | Self::Http(500..=599))
    }
}

pub(crate) fn transport_loss(error: &anyhow::Error) -> bool {
    matches!(classify(error), Failure::NotDelivered | Failure::Unanswered)
}

impl RpcError {
    pub(crate) fn qr_expired(&self) -> bool {
        self.gateway == 10052 && self.status == 410
    }
    pub(crate) fn certificate_missing(&self) -> bool {
        self.http == 400
    }
    pub(crate) fn needs_refresh(&self) -> bool {
        self.code == 119
    }
    pub(crate) fn retry_refresh(&self) -> bool {
        self.gateway == 10202
    }
    pub(crate) fn refresh_rejected(&self) -> bool {
        self.needs_refresh() || self.gateway == 10201
    }
}

pub(crate) fn saved_session_failure(error: &anyhow::Error) -> anyhow::Result<SignInFailure> {
    if let Some(reason) = error.downcast_ref::<SignInFailure>() {
        return Ok(reason.clone());
    }
    if error.chain().any(|cause| {
        cause.is::<Failure>() || cause.is::<RpcError>() || cause.is::<TransportFailure>()
    }) {
        return Err(anyhow::Error::new(classify(error)));
    }
    Ok(SignInFailure::SavedSession)
}

pub(crate) fn diagnostic(error: &(dyn std::error::Error + 'static)) -> Option<String> {
    if let Some(rpc) = error.downcast_ref::<RpcError>() {
        return Some(format!(
            "rpc:{}:http={}:gateway={}:code={}",
            rpc.method.rsplit('/').next().unwrap_or(&rpc.method),
            rpc.http,
            rpc.gateway,
            rpc.code
        ));
    }
    if let Some(failure) = error.downcast_ref::<Failure>() {
        return Some(match failure {
            Failure::Bug(label) => format!("client-bug:{label}"),
            _ => failure.to_string(),
        });
    }
    error
        .downcast_ref::<SignInFailure>()
        .map(|reason| format!("sign-in:{}", reason.kind()))
}

pub(super) fn phone_error(error: anyhow::Error) -> anyhow::Error {
    if error.downcast_ref::<RpcError>().is_some() {
        error.context(SignInFailure::Phone)
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classification_uses_the_first_matching_cause() {
        let rpc = RpcError {
            gateway: 0,
            method: "test".into(),
            http: 503,
            code: 0,
            status: 0,
        };
        let error = TransportFailure {
            write_started: false,
            source: TransportError::Io(std::io::Error::other(rpc)),
        };
        assert_eq!(
            classify(&anyhow::Error::new(error).context("outer")),
            Failure::NotDelivered
        );
        // Policy failures nested in IO are not promoted to top-level overrides.
        let nested = anyhow::Error::new(std::io::Error::other(Failure::NotDelivered));
        assert!(matches!(classify(&nested), Failure::Bug(_)));
    }

    #[test]
    fn send_retry_is_limited_to_safe_failures() {
        assert!(Failure::NotDelivered.retry_send());
        assert!(!Failure::Unanswered.retry_send());
        for (refusal, expected) in [
            (ServerRefusal::Talk(115), true),
            (ServerRefusal::Gateway(UNKNOWN_GATEWAY), true),
            (ServerRefusal::Http(408), false),
            (ServerRefusal::Http(429), false),
            (ServerRefusal::Http(503), false),
            (ServerRefusal::Talk(13), false),
        ] {
            assert_eq!(
                Failure::Refused(Refusal::Server(refusal)).retry_send(),
                expected
            );
        }
        let error =
            super::super::decode_response("/test", 200, br#"{"code":0,"message":"unexpected"}"#)
                .unwrap_err();
        assert_eq!(
            classify(&error),
            Failure::Refused(Refusal::Server(ServerRefusal::Gateway(UNKNOWN_GATEWAY)))
        );
    }
    #[test]
    fn extension_response_rules() {
        for (code, reason) in [
            (1, SignOutReason::AuthenticationFailed),
            (7, SignOutReason::NotAvailableUser),
            (8, SignOutReason::NotAuthorizedDevice),
        ] {
            let error = super::super::decode_response(
                "/test",
                200,
                &json!({"code":10051,"data":{"code":code}})
                    .to_string()
                    .into_bytes(),
            )
            .unwrap_err();
            assert_eq!(classify(&error), Failure::SignedOut(reason));
        }
        let rpc = |gateway, code| RpcError {
            gateway,
            code,
            method: "test".into(),
            http: 200,
            status: 0,
        };
        assert!(rpc(10051, 119).needs_refresh());
        assert!(rpc(10201, 0).refresh_rejected());
        assert!(rpc(10202, 0).retry_refresh());
        assert!(classify(&rpc(10051, 115).into()).retry_read());
        assert_eq!(
            classify(&rpc(10051, 13).into()),
            Failure::Refused(Refusal::Server(ServerRefusal::Talk(13)))
        );
    }

    #[test]
    fn missing_access_token_returns_signed_out_without_a_request() {
        let module = crate::test_support::installed_module();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let wasm = module.read().unwrap();
        let mut api = super::super::Api::new(&wasm, &module.version).unwrap();
        api.http = super::super::http::Transport::local(
            "synthetic",
            format!("http://{}", listener.local_addr().unwrap()),
        );
        for token in [serde_json::Value::Null, json!("")] {
            api.state.token_v3_issue_result.access_token =
                token.as_str().unwrap_or_default().into();
            assert_eq!(
                classify(&api.call("/api/test", &json!([]), "", 0).unwrap_err()),
                Failure::SignedOut(SignOutReason::NoSession)
            );
        }
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn second_119_ends_the_session() {
        let module = crate::test_support::installed_module();

        use crate::client::http::tests::{accept, local_server};
        use std::io::{BufRead, BufReader, Read, Write};
        let (http, server) = local_server(|listener| {
            for (path, response) in [
                ("/api/test", json!({"code":10051,"data":{"code":119}})),
                (
                    "/api/auth/tokenRefresh",
                    json!({"code":0,"message":"OK","data":{"accessToken":"renewed"}}),
                ),
                ("/api/test", json!({"code":10051,"data":{"code":119}})),
            ] {
                let mut socket = BufReader::new(accept(&listener));
                let mut line = String::new();
                socket.read_line(&mut line).unwrap();
                assert_eq!(line, format!("POST {path} HTTP/1.1\r\n"));
                let mut length = 0;
                loop {
                    line.clear();
                    socket.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                socket.read_exact(&mut body).unwrap();
                let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(
                    body,
                    if path == "/api/test" {
                        json!([])
                    } else {
                        json!({"refreshToken":"refresh"})
                    }
                );
                let body = response.to_string();
                write!(
                    socket.get_mut(),
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let wasm = module.read().unwrap();
        let mut api = super::super::Api::new(&wasm, &module.version).unwrap();
        api.http = http;
        api.state.token_v3_issue_result = super::super::Tokens {
            access_token: "old".into(),
            refresh_token: Some("refresh".into()),
            rest: serde_json::from_value(json!({"expiresIn":123,"extra":{"a":true}})).unwrap(),
        };
        let error = api.call("/api/test", &json!([]), "", 0).unwrap_err();
        assert_eq!(
            classify(&error),
            Failure::SignedOut(SignOutReason::TokenRefresh)
        );
        assert_eq!(
            api.state.token_v3_issue_result.refresh_token.as_deref(),
            Some("refresh")
        );
        assert_eq!(api.state.token_v3_issue_result.access_token, "renewed");
        assert_eq!(
            serde_json::to_value(&api.state.token_v3_issue_result).unwrap(),
            json!({"accessToken":"renewed","refreshToken":"refresh","expiresIn":123,"extra":{"a":true}})
        );
        server.join().unwrap();
    }
}

#[cfg(test)]
mod top_level_tests {
    use super::*;

    #[test]
    fn refresh_checks_do_not_follow_io_sources() {
        let rpc = || RpcError {
            gateway: 0,
            method: "test".into(),
            http: 200,
            code: 119,
            status: 0,
        };
        assert!(needs_refresh::<()>(&Err(rpc().into())));
        let wrapped = std::io::Error::other(rpc());
        assert!(!needs_refresh::<()>(&Err(wrapped.into())));
    }

    #[test]
    fn media_and_missing_file_checks_do_not_follow_io_sources() {
        use super::super::desktop::assets::Failure as Media;
        assert_eq!(
            media_failure(&Media::UnsupportedHost.into()),
            Media::UnsupportedHost
        );
        let wrapped = std::io::Error::other(Media::UnsupportedHost);
        assert_eq!(media_failure(&wrapped.into()), Media::Decode);
        let missing = || std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(media_failure(&missing().into()), Media::Http(404));
        let wrapped = std::io::Error::other(missing());
        assert_eq!(media_failure(&wrapped.into()), Media::Decode);
    }
}

#[cfg(test)]
pub(super) mod wire_fixture {
    pub const KEY_ROTATED: i64 = 82;
    pub const SENDER_KEY_STALE: i64 = 83;
    pub const GROUP_KEY_ROTATED: i64 = 84;
    pub fn talk(code: i64) -> serde_json::Value {
        serde_json::json!({"code":10051,"data":{"code":code}})
    }
    pub fn refused_key() -> serde_json::Value {
        talk(13)
    }
    pub fn signed_out() -> serde_json::Value {
        talk(1)
    }
    pub fn qr_expired() -> serde_json::Value {
        serde_json::json!({"code":10052,"data":{"statusCode":410}})
    }
}

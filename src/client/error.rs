//! The single network-policy boundary.
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
/// Local sign-in prerequisites are independent of network policy.
#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("sign-in failed: {0:?}")]
    SignIn(SignInFailure),
}
/// Transport evidence is recorded before its request scope is released.
#[derive(Debug, thiserror::Error)]
#[error("HTTPS transport failed; response not logged")]
pub(crate) struct TransportFailure {
    pub write_started: bool,
    pub source: ureq::Error,
}

pub fn classify(error: &anyhow::Error) -> Failure {
    let mut failure = classification(error);
    if let Failure::Bug(message) = &mut failure {
        for context in error.chain().take_while(|cause| cause.source().is_some()) {
            message.push_str(": ");
            message.push_str(&context.to_string());
        }
    }
    failure
}

fn classification(error: &anyhow::Error) -> Failure {
    if let Some(Error::SignIn(reason)) = error.downcast_ref::<Error>() {
        return Failure::SignedOut(SignOutReason::Unavailable(reason.clone()));
    }
    if let Some(failure) = error.downcast_ref::<Failure>() {
        return failure.clone();
    }
    for cause in error.chain() {
        if let Some(error) = cause.downcast_ref::<TransportFailure>().or_else(|| {
            cause
                .downcast_ref::<std::io::Error>()?
                .get_ref()?
                .downcast_ref::<TransportFailure>()
        }) {
            return match error.source {
                ureq::Error::Io(_)
                | ureq::Error::Timeout(_)
                | ureq::Error::ConnectionFailed
                | ureq::Error::HostNotFound
                | ureq::Error::Tls(_) => {
                    if error.write_started {
                        Failure::Unanswered
                    } else {
                        Failure::NotDelivered
                    }
                }
                _ => Failure::Bug("HTTP protocol failure".into()),
            };
        }
        if let Some(error) = cause.downcast_ref::<crate::client::RpcError>() {
            if error.gateway == 99999 {
                return Failure::Refused(Refusal::Gateway(99999));
            }
            return match error.code {
                1 => Failure::SignedOut(SignOutReason::AuthenticationFailed),
                7 => Failure::SignedOut(SignOutReason::NotAvailableUser),
                8 => Failure::SignedOut(SignOutReason::NotAuthorizedDevice),
                115 => Failure::Refused(Refusal::Talk(115)),
                code if code != 0 => Failure::Refused(Refusal::Talk(code)),
                _ if error.http != 200 => Failure::Refused(Refusal::Http(error.http)),
                _ if error.gateway != 0 => Failure::Refused(Refusal::Gateway(error.gateway)),
                _ => Failure::Refused(Refusal::Gateway(99999)),
            };
        }
        if let Some(error) = cause.downcast_ref::<super::desktop::assets::Failure>() {
            return match error {
                super::desktop::assets::Failure::Network
                | super::desktop::assets::Failure::Timeout => Failure::Unanswered,
                super::desktop::assets::Failure::Http(code) => {
                    Failure::Refused(Refusal::Http(*code))
                }
                _ => Failure::Bug("media response failure".into()),
            };
        }
    }
    Failure::Bug("decode, crypto, protocol or invariant failure".into())
}

/// Stable rejection categories; diagnostic errors never become row text.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
pub enum Refusal {
    #[error("internal error")]
    Internal,
    #[error("HTTP {0}")]
    Http(u16),
    #[error("gateway code {0}")]
    Gateway(i64),
    #[error("TalkException code {0}")]
    Talk(i64),
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

/// Prerequisites and protocol outcomes carried intact to the sign-in surface.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    SavedSession(SavedSessionFailure),
    Expired,
    SessionIdentity,
    Internal,
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
            Self::SavedSession(_) => "saved-session",
            Self::Internal => "internal",
            Self::Expired => "expired",
            Self::SessionIdentity => "session-identity",
        }
    }
}

/// Local saved-session failures, independent of LINE authentication.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SavedSessionFailure {
    #[error("saved-session: {self:?}")]
    Keys,
    #[error("saved-session: {self:?}")]
    Corrupt,
    #[error("saved-session: {self:?}")]
    CredentialsStore,
    #[error("saved-session: {self:?}")]
    Keyring,
    #[error("saved-session: {self:?}")]
    Io,
    #[error("saved-session: {self:?}")]
    ImageCache,
}
impl SavedSessionFailure {
    pub(crate) fn error(self) -> Error {
        Error::SignIn(SignInFailure::SavedSession(self))
    }
}

impl super::Api {
    pub(super) fn call(
        &mut self,
        path: &str,
        args: &serde_json::Value,
        session: &str,
        poll_seconds: u64,
    ) -> anyhow::Result<serde_json::Value> {
        let token = super::access(&self.state)
            .map_err(|_| Failure::SignedOut(SignOutReason::NoSession))?
            .to_owned();
        let result = self.call_once(path, args, &token, session, poll_seconds);
        if !needs_refresh(&result) {
            return result;
        }
        self.refresh_session()?;
        let token = super::access(&self.state)?.to_owned();
        let result = self.call_once(path, args, &token, session, poll_seconds);
        if needs_refresh(&result) {
            Err(Failure::SignedOut(SignOutReason::TokenRefresh).into())
        } else {
            result
        }
    }
    pub(super) fn refresh_session(&mut self) -> anyhow::Result<()> {
        let token = self.state["token_v3_issue_result"]["refreshToken"]
            .as_str()
            .filter(|token| !token.is_empty())
            .ok_or(Failure::SignedOut(SignOutReason::TokenRefresh))?
            .to_owned();
        let access = super::access(&self.state)?.to_owned();
        let args = serde_json::json!({"refreshToken": token});
        let result = self.call_once("/api/auth/tokenRefresh", &args, &access, "", 0);
        let result = if result
            .as_ref()
            .err()
            .and_then(|error| error.downcast_ref::<super::RpcError>())
            .is_some_and(|error| error.gateway == 10202)
        {
            self.call_once("/api/auth/tokenRefresh", &args, &access, "", 0)
        } else {
            result
        };
        let value = result.map_err(|error| {
            if error
                .downcast_ref::<super::RpcError>()
                .is_some_and(|error| error.code == 119 || error.gateway == 10201)
            {
                anyhow::Error::new(Failure::SignedOut(SignOutReason::TokenRefresh))
            } else {
                error
            }
        })?;
        let access = value["accessToken"]
            .as_str()
            .filter(|token| !token.is_empty())
            .ok_or(Failure::Bug("refresh missing access token".into()))?
            .to_owned();
        self.state["token_v3_issue_result"]
            .as_object_mut()
            .ok_or(Failure::Bug("saved token".into()))?
            .extend(
                value
                    .as_object()
                    .ok_or(Failure::Bug("refresh token response".into()))?
                    .clone(),
            );
        if let Some(path) = &self.session_path {
            super::save_state(path, &self.state)?;
        }
        *self.stream_access.write().unwrap() = access;
        Ok(())
    }
}
pub(super) fn needs_refresh<T>(result: &anyhow::Result<T>) -> bool {
    result
        .as_ref()
        .err()
        .and_then(|error| error.downcast_ref::<super::RpcError>())
        .is_some_and(|error| error.code == 119)
}

pub(crate) fn decode_response(
    path: &str,
    http: u16,
    content_type: &str,
    bytes: &[u8],
) -> anyhow::Result<serde_json::Value> {
    let response: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(response) => response,
        Err(_) if http != 200 => {
            return Err(super::RpcError {
                gateway: 0,
                method: path.rsplit('/').next().unwrap_or("RPC").into(),
                http,
                code: 0,
                status: 0,
            }
            .into());
        }
        Err(_) => return Err(Failure::Bug(format!(
            "response decode failed: method={} status={http} content-type={content_type} body-length={}",
            path.rsplit('/').next().unwrap_or("RPC"), bytes.len()
        )).into()),
    };
    if http == 200 && response["message"] == "OK" {
        return Ok(response
            .get("data")
            .cloned()
            .unwrap_or(serde_json::Value::Null));
    }
    let gateway = response["code"]
        .as_i64()
        .unwrap_or(if http == 200 { 99999 } else { 0 });
    let code = if gateway == 10051 {
        response["data"]["code"]
            .as_i64()
            .ok_or(Failure::Bug("missing TalkException code".into()))?
    } else {
        0
    };
    Err(super::RpcError {
        gateway,
        method: path.rsplit('/').next().unwrap_or("RPC").into(),
        http,
        code,
        status: response["data"]["statusCode"].as_i64().unwrap_or(0),
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn response_code_table() {
        let signed_out = [
            (1, SignOutReason::AuthenticationFailed),
            (7, SignOutReason::NotAvailableUser),
            (8, SignOutReason::NotAuthorizedDevice),
        ];
        for (code, reason) in signed_out {
            let response = json!({"code": 10051, "data": {"code": code}}).to_string();
            assert_eq!(
                classify(&decode_response("/api/test", 200, "", response.as_bytes()).unwrap_err()),
                Failure::SignedOut(reason)
            );
        }
        for code in [13, 14, 17, 43, 44, 119, 120, 503] {
            let response = json!({"code": 10051, "data": {"code": code}}).to_string();
            assert_eq!(
                classify(&decode_response("/api/test", 200, "", response.as_bytes()).unwrap_err()),
                Failure::Refused(Refusal::Talk(code))
            );
        }
        for code in [503, 10105, 10201, 10202, 99999] {
            let response = json!({"code":code});
            assert_eq!(
                classify(
                    &decode_response("/api/test", 200, "", response.to_string().as_bytes())
                        .unwrap_err()
                ),
                Failure::Refused(Refusal::Gateway(code))
            );
        }
        {
            let response = json!({"code":10051,"data":{"code":115}});
            assert_eq!(
                classify(
                    &decode_response("/api/test", 200, "", response.to_string().as_bytes())
                        .unwrap_err()
                ),
                Failure::Refused(Refusal::Talk(115))
            );
        }
        assert_eq!(
            classify(&decode_response("/api/test", 200, "", b"{}").unwrap_err()),
            Failure::Refused(Refusal::Gateway(99999))
        );
        for status in [400, 401, 403, 408, 429, 500, 503] {
            assert_eq!(
                classify(
                    &decode_response("/api/test", status, "", b"gateway unavailable").unwrap_err()
                ),
                Failure::Refused(Refusal::Http(status))
            );
            assert_eq!(
                classify(&decode_response("/api/test", status, "", b"{}").unwrap_err()),
                Failure::Refused(Refusal::Http(status))
            );
        }
        for write_started in [false, true] {
            let error = TransportFailure {
                write_started,
                source: ureq::Error::Io(std::io::ErrorKind::ConnectionReset.into()),
            };
            assert_eq!(
                classify(&error.into()),
                if write_started {
                    Failure::Unanswered
                } else {
                    Failure::NotDelivered
                }
            );
        }
        assert_eq!(
            classify(&decode_response("/api/test", 200, "text/html", b"private body").unwrap_err()),
            Failure::Bug("response decode failed: method=test status=200 content-type=text/html body-length=12".into())
        );
    }

    #[test]
    fn missing_access_token_returns_signed_out_without_a_request() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let wasm = crate::test_support::read_fixture(&crate::test_support::fixture_path()).unwrap();
        let mut api = super::super::Api::new(&wasm, crate::test_support::FIXTURE_VERSION).unwrap();
        api.http = super::super::http::Transport::local(
            "synthetic",
            format!("http://{}", listener.local_addr().unwrap()),
        );
        for token in [serde_json::Value::Null, json!("")] {
            api.state = json!({"token_v3_issue_result":{"accessToken":token}});
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
        let wasm = crate::test_support::read_fixture(&crate::test_support::fixture_path()).unwrap();
        let mut api = super::super::Api::new(&wasm, crate::test_support::FIXTURE_VERSION).unwrap();
        api.http = http;
        api.state = json!({"token_v3_issue_result":{"accessToken":"old","refreshToken":"refresh"}});
        let error = api.call("/api/test", &json!([]), "", 0).unwrap_err();
        assert_eq!(
            classify(&error),
            Failure::SignedOut(SignOutReason::TokenRefresh)
        );
        server.join().unwrap();
    }
}

#[derive(Debug, thiserror::Error)]
#[error("HTTPS request cancelled")]
pub(crate) struct Cancelled;

impl std::fmt::Display for SignInFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExtensionMissing => f.write_str("the LINE extension is not installed"),
            Self::CryptoChanged { .. } => f.write_str("the crypto module has changed"),
            Self::CryptoModule(_) => f.write_str("the crypto module is unavailable"),
            Self::StateDirectory(_) => f.write_str("the session directory is not writable"),
            Self::Keyring => f.write_str("the keyring is unavailable"),
            Self::Network => f.write_str("LINE could not be reached"),
            Self::Server => f.write_str("LINE refused sign-in"),
            Self::Phone => f.write_str("phone approval was not received"),
            Self::Cancelled => f.write_str("sign-in was cancelled"),
            Self::SavedSession(reason) => write!(f, "{reason}"),
            Self::Expired => f.write_str("sign-in expired"),
            Self::SessionIdentity => f.write_str("the session belongs to another account"),
            Self::Internal => f.write_str("the saved session could not be opened"),
        }
    }
}

pub(super) fn sign_in_failure(error: &anyhow::Error, cancelled: bool) -> SignInFailure {
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

impl wasmi::core::HostError for Error {}

impl Failure {
    pub fn retry_read(&self) -> bool {
        matches!(
            self,
            Self::NotDelivered
                | Self::Unanswered
                | Self::Refused(
                    Refusal::Talk(115)
                        | Refusal::Gateway(99999)
                        | Refusal::Http(408 | 429 | 500..=599)
                )
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
    match classification(error) {
        Failure::NotDelivered | Failure::Unanswered => {
            if error
                .downcast_ref::<TransportFailure>()
                .is_some_and(|error| matches!(error.source, ureq::Error::Timeout(_)))
            {
                Media::Timeout
            } else {
                Media::Network
            }
        }
        Failure::Refused(Refusal::Http(code)) => Media::Http(code),
        _ => Media::Decode,
    }
}

pub fn media_retry(failure: super::desktop::assets::Failure, automatic: bool) -> bool {
    use super::desktop::assets::Failure as Media;
    match failure {
        Media::Network | Media::Timeout | Media::Http(500..=599) => true,
        Media::Cancelled | Media::Http(403 | 408 | 429) => !automatic,
        _ => false,
    }
}

pub(crate) fn transport_loss(error: &anyhow::Error) -> bool {
    matches!(
        classification(error),
        Failure::NotDelivered | Failure::Unanswered
    )
}

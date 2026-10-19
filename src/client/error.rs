//! Errors at the RPC boundary, before presentation text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetCause {
    Unreachable,
}
#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("sign-in failed: {0:?}")]
    SignIn(SignInFailure),
    #[error("internal network worker error")]
    Internal,
    #[error("offline")]
    Offline(NetCause),
    #[error("authorization expired")]
    Auth,
    #[error("session identity changed")]
    SessionInvalid,
    #[error("server error {0}")]
    Server(i64),
    #[error("send rejected: {0}")]
    Rejected(RejectReason),
    #[error("response decode failed: {0}")]
    Decode(String),
    #[error("protocol error")]
    Protocol,
    #[error("crypto failed")]
    Crypto,
}

/// Stable rejection categories; diagnostic errors never become row text.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
pub enum RejectReason {
    #[error("Upload stalled")]
    UploadStalled,
    #[error("Server refused the file")]
    UploadRefused,
    #[error("Upload failed")]
    UploadFailed,
    #[error("File missing")]
    FileMissing,
    #[serde(rename = "Synthetic send failure")]
    #[error("rejection: {self:?}")]
    SyntheticSend,
    #[serde(rename = "Synthetic friends failure")]
    #[error("rejection: {self:?}")]
    SyntheticFriends,
    #[serde(rename = "Synthetic sign-in is unavailable")]
    #[error("rejection: {self:?}")]
    SyntheticSignIn,
    #[serde(rename = "read failed")]
    #[error("rejection: {self:?}")]
    ReadFailed,
    #[serde(rename = "Synthetic unsend rejection")]
    #[error("rejection: {self:?}")]
    SyntheticUnsend,
    #[serde(other)]
    #[serde(rename = "Message was not accepted")]
    #[error("rejection: {self:?}")]
    NotAccepted,
}

/// Preserve boundary errors; unknown local failures are decode errors.
pub(crate) fn classify(error: &anyhow::Error) -> Error {
    if let Some(error) = error.downcast_ref::<Error>() {
        return error.clone();
    }
    if let Some(error) = error.downcast_ref::<crate::client::RpcError>() {
        return if matches!(error.http, 401 | 403) {
            Error::Auth
        } else if (500..600).contains(&error.http) {
            Error::Server(i64::from(error.http))
        } else {
            Error::Server(error.code)
        };
    }
    Error::Decode(error.to_string())
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

impl wasmi::core::HostError for Error {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn media_rejections_keep_their_wording_through_classification() {
        for (reason, text) in [
            (RejectReason::UploadStalled, "Upload stalled"),
            (RejectReason::UploadRefused, "Server refused the file"),
            (RejectReason::UploadFailed, "Upload failed"),
        ] {
            let error = anyhow::Error::new(Error::Rejected(reason)).context("upload worker");
            let Error::Rejected(reason) = classify(&error) else {
                panic!("lost upload reason")
            };
            assert_eq!(reason.to_string(), text);
        }
    }
}

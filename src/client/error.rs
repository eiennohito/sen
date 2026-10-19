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
    #[error("response decode failed")]
    Decode,
    #[error("protocol error")]
    Protocol,
    #[error("crypto failed")]
    Crypto,
}

/// Stable rejection categories; diagnostic errors never become row text.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
pub enum RejectReason {
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

/// Preserve source categories; unclassified response failures are decode errors.
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
    Error::Decode
}

/// Prerequisites and protocol outcomes carried intact to the sign-in surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignInFailure {
    CryptoModule(std::path::PathBuf),
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
    pub fn kind(&self) -> &'static str {
        match self {
            Self::CryptoModule(_) => "crypto-module",
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
    #[error("the saved crypto version is unsupported")]
    Version,
    #[error("Letter Sealing keys are missing or do not match")]
    Keys,
    #[error("the saved data is corrupt")]
    Corrupt,
    #[error("the credentials store does not match")]
    CredentialsStore,
    #[error("the keyring is locked or unavailable, or its key is missing")]
    Keyring,
    #[error("the saved session could not be read or written")]
    Io,
    #[error("the image cache could not be opened")]
    ImageCache,
}
impl SavedSessionFailure {
    pub(crate) fn error(self) -> Error {
        Error::SignIn(SignInFailure::SavedSession(self))
    }
}

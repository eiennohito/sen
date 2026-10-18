//! Errors at the RPC boundary, before presentation text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetCause {
    Unreachable,
    Server(i64),
    Protocol,
}
#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("offline")]
    Offline(NetCause),
    #[error("authorization expired")]
    Auth,
    #[error("session identity changed")]
    SessionInvalid,
    #[error("server error {0}")]
    Server(i64),
    #[error("send rejected: {0}")]
    Rejected(String),
    #[error("response decode failed")]
    Decode,
    #[error("crypto failed")]
    Crypto,
}

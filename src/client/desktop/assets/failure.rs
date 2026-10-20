use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Failure {
    Timeout,
    Network,
    Http(u16),
    UnsupportedHost,
    RejectedPath,
    TooLarge,
    Decode,
    Cancelled,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "media failure: {self:?}")
    }
}
impl std::error::Error for Failure {}

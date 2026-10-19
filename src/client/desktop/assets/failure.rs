use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

impl Failure {
    pub fn from_error(error: &anyhow::Error) -> Self {
        error
            .downcast_ref::<Self>()
            .copied()
            .or_else(|| {
                error
                    .downcast_ref::<crate::client::error::Cancelled>()
                    .map(|_| Self::Cancelled)
            })
            .or_else(|| {
                error
                    .downcast_ref::<crate::client::error::TransportFailure>()
                    .map(|error| match error.source {
                        ureq::Error::Timeout(_) => Self::Timeout,
                        _ => Self::Network,
                    })
            })
            .or_else(|| {
                error
                    .downcast_ref::<std::io::Error>()
                    .filter(|error| error.kind() == std::io::ErrorKind::NotFound)
                    .map(|_| Self::Http(404))
            })
            .unwrap_or(Self::Network)
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "media failure: {self:?}")
    }
}
impl std::error::Error for Failure {}

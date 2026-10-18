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
    pub fn retriable(self) -> bool {
        matches!(
            self,
            Self::Timeout
                | Self::Network
                | Self::Cancelled
                | Self::Http(403 | 408 | 429 | 500..=599)
        )
    }

    pub fn automatic(self) -> bool {
        matches!(self, Self::Timeout | Self::Network | Self::Http(500..=599))
    }

    pub fn image_text(self) -> &'static str {
        match self {
            Self::Timeout => "Image took too long to load",
            Self::Network => "Couldn’t load this image",
            Self::Http(404) => "Not available for this sender",
            Self::Http(403) => "You don’t have access to this image",
            Self::Http(500..=599) => "Image is temporarily unavailable",
            Self::Http(_) => "Image is unavailable",
            Self::UnsupportedHost => "Not available for this sender",
            Self::RejectedPath => "Image is unavailable",
            Self::TooLarge => "Image too large",
            Self::Decode => "Couldn’t display this image",
            Self::Cancelled => "Image loading cancelled",
        }
    }

    pub fn file_text(self) -> &'static str {
        match self {
            Self::Timeout => "File took too long to download",
            Self::Network => "Couldn’t download this file",
            Self::Http(404) => "File is no longer available",
            Self::Http(403) => "You don’t have access to this file",
            Self::Http(500..=599) => "File is temporarily unavailable",
            Self::Http(_) | Self::RejectedPath => "File is unavailable",
            Self::UnsupportedHost => "Not available for this sender",
            Self::TooLarge => "File too large",
            Self::Decode => "Couldn’t open this file",
            Self::Cancelled => "File download cancelled",
        }
    }

    pub(crate) fn transport_error(self) -> anyhow::Error {
        TransportFailure(self).into()
    }

    pub fn from_error(error: &anyhow::Error) -> Self {
        error
            .downcast_ref::<Self>()
            .copied()
            .or_else(|| {
                error
                    .downcast_ref::<TransportFailure>()
                    .map(|error| error.0)
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

#[derive(Debug)]
struct TransportFailure(Failure);
impl fmt::Display for TransportFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            Failure::Cancelled => "HTTPS request cancelled",
            Failure::Timeout => "HTTPS request timed out; response not logged",
            _ => "HTTPS transport failed; response not logged",
        })
    }
}
impl std::error::Error for TransportFailure {}

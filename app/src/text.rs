//! User-facing wording for typed library outcomes.
use line::client::{desktop::assets::Failure, error::Refusal};

pub(crate) fn image_text(failure: Failure) -> &'static str {
    match failure {
        Failure::Timeout => "Image took too long to load",
        Failure::Network => "Couldn’t load this image",
        Failure::Http(404) => "Image is no longer available",
        Failure::Http(403) => "You don’t have access to this image",
        Failure::Http(500..=599) => "Image is temporarily unavailable",
        Failure::Http(_) => "Image is unavailable",
        Failure::UnsupportedHost => "Not available for this sender",
        Failure::RejectedPath => "Image is unavailable",
        Failure::TooLarge => "Image too large",
        Failure::Decode => "Couldn’t display this image",
        Failure::Cancelled => "Image loading cancelled",
    }
}

pub(crate) fn rejection(reason: &Refusal) -> String {
    match reason {
        Refusal::Server(line::client::error::ServerRefusal::Http(code)) => {
            return format!("HTTP {code}");
        }
        Refusal::Server(line::client::error::ServerRefusal::Gateway(code)) => {
            return format!("gateway code {code}");
        }
        Refusal::Server(line::client::error::ServerRefusal::Talk(code)) => {
            return format!("server code {code}");
        }
        Refusal::SignedOut => "signed out",
        Refusal::Unanswered => "no answer received",
        Refusal::AppClosed => "app closed before sending",
        Refusal::Internal => "internal error",
        Refusal::FileMissing => "File missing",
        Refusal::NotAccepted => "Message was not accepted",
    }
    .into()
}

pub(crate) fn create_failure(failure: &line::engine::CreateFailure) -> &str {
    use line::engine::CreateFailure;
    match failure {
        CreateFailure::InvalidSelection => "Select at least two friends to create a group.",
        CreateFailure::Local => "Couldn't create group. Try again.",
        CreateFailure::Rejected => "Couldn't create group. Try again.",
        CreateFailure::Uncertain => {
            "The group may have been created. Check your conversations before trying again."
        }
    }
}

pub(crate) fn profile_failure(failure: line::engine::ProfileFailure) -> &'static str {
    use line::engine::ProfileFailure;
    match failure {
        ProfileFailure::Rename => "Couldn't rename.",
        ProfileFailure::AddFriend => "Couldn't add friend.",
        ProfileFailure::Mute => "Couldn't change notifications.",
        ProfileFailure::Leave => "Couldn't leave group.",
    }
}

pub(crate) fn pairing_stage(stage: line::client::pairing::Stage) -> &'static str {
    use line::client::pairing::Stage;
    match stage {
        Stage::Authenticated | Stage::Synthetic => SIGNED_IN,
        Stage::WaitingKeyring | Stage::CreatingSession => PREPARING_SIGN_IN,
        Stage::AwaitScan | Stage::CheckingCertificate | Stage::AwaitPin => WAITING_FOR_PHONE,
        Stage::RequestCredentials
        | Stage::UnwrappingKeys
        | Stage::KeysVerified
        | Stage::CheckingKeys => SIGNING_IN,
    }
}

pub(crate) const SIGNED_IN: &str = "Signed in";
pub(crate) const SIGNING_IN: &str = "Signing in…";
pub(crate) const WAITING_FOR_PHONE: &str = "Waiting for your phone…";
pub(crate) const PREPARING_SIGN_IN: &str = "Preparing sign-in…";
pub(crate) const CANCELLING_SIGN_IN: &str = "Cancelling sign-in…";

pub fn storage_failure(reason: line::engine::StorageFailure) -> &'static str {
    use line::engine::StorageFailure::*;
    match reason {
        Full => "Can't save messages: disk is full. New messages are shown but won't be kept.",
        ReadOnly => {
            "Can't save messages: storage is read-only. New messages are shown but won't be kept."
        }
        Permission => {
            "Can't save messages: permission denied. New messages are shown but won't be kept."
        }
        Io => "Can't save messages: disk I/O failed. New messages are shown but won't be kept.",
    }
}
pub fn sign_in_failure(reason: &line::client::error::SignInFailure) -> String {
    use line::client::error::SignInFailure::*;
    match reason {
        CryptoModule(path) => format!("Can't load the LINE crypto module at {}. Check LINE for Chrome or the developer override.", path.display()),
        ExtensionMissing => "Install LINE for Chrome — this app uses its crypto module. Searched Chrome and Chromium profiles under $XDG_CONFIG_HOME (default $HOME/.config).".into(),
        CryptoChanged { version, missing } => format!("LINE for Chrome {version} changed its crypto module (missing {missing}); this app doesn't support it yet"),
        StateDirectory(path) => format!("State directory is not writable: {}. Grant write permission and try again.", path.display()),
        Keyring => "Unlock your keyring (KWallet/GNOME Keyring) and try again. Ensure Secret Service is running with a default keyring.".into(),
        Network => "Can't reach LINE. Check your network connection and try again.".into(),
        Server => "LINE refused sign-in. Try again later.".into(),
        Phone => "Phone approval was rejected or timed out. Start again and approve sign-in on your phone.".into(),
        Cancelled => String::new(),
        SavedSession => "Your saved sign-in file is corrupt or unreadable. Restore a backup or sign in again.".into(),
        SessionStoreMismatch => "Your saved sign-in uses a different credential store. Select the original credential store and try again.".into(),
        SessionKeyMissing => "Unlock your keyring and try again. This saved sign-in needs its original keyring item; restore it if missing.".into(),
        SessionIo => "Can't read or write your saved sign-in file. Check file permissions and storage, then try again.".into(),
        Expired => "Sign-in expired. Sign in again.".into(),
        SessionIdentity => "The saved sign-in belongs to a different account. Sign in again.".into(),
        Internal => "Sign-in stopped because of an internal error. See the log and try again.".into(),
    }
}

pub(crate) const LINK_FAILURE: &str = "Couldn't open link";

pub(crate) fn member_name(name: Option<&str>) -> &str {
    name.unwrap_or("Unknown member")
}
pub(crate) fn friend_name(friend: &line::store::Friend) -> &str {
    if friend.group {
        group_name(friend.name.as_deref())
    } else {
        friend.name.as_deref().unwrap_or("Unnamed friend")
    }
}
pub(crate) fn group_name(name: Option<&str>) -> &str {
    name.unwrap_or("Unnamed group")
}

pub fn sign_out_reason(reason: &line::client::error::SignOutReason) -> String {
    use line::client::error::SignOutReason::*;
    match reason {
        Unavailable(reason) => sign_in_failure(reason),
        NoSession => "Sign in to connect to LINE.".into(),
        AuthenticationFailed => "Sign-in expired. Sign in again.".into(),
        NotAvailableUser => "This LINE account is unavailable.".into(),
        NotAuthorizedDevice => "This device is no longer authorized. Sign in again.".into(),
        TokenRefresh => "Your sign-in could not be renewed. Sign in again.".into(),
        IdentityChanged => {
            "The saved sign-in belongs to a different account. Sign in again.".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use line::client::pairing::Stage;

    #[test]
    fn pairing_status_follows_phone_approval() {
        for stage in [Stage::WaitingKeyring, Stage::CreatingSession] {
            assert_eq!(pairing_stage(stage), PREPARING_SIGN_IN);
        }
        for stage in [
            Stage::AwaitScan,
            Stage::CheckingCertificate,
            Stage::AwaitPin,
        ] {
            assert_eq!(pairing_stage(stage), WAITING_FOR_PHONE);
        }
        for stage in [
            Stage::RequestCredentials,
            Stage::UnwrappingKeys,
            Stage::CheckingKeys,
            Stage::KeysVerified,
        ] {
            assert_eq!(pairing_stage(stage), SIGNING_IN);
        }
        for stage in [Stage::Authenticated, Stage::Synthetic] {
            assert_eq!(pairing_stage(stage), SIGNED_IN);
        }
    }
}

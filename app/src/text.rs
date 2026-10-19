//! User-facing wording for typed library outcomes.
use line_client::client::{desktop::assets::Failure, error::RejectReason};

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

pub(crate) fn rejection(reason: &RejectReason) -> &'static str {
    match reason {
        RejectReason::UploadStalled => "Upload stalled",
        RejectReason::UploadRefused => "Server refused the file",
        RejectReason::UploadFailed => "Upload failed",
        RejectReason::FileMissing => "File missing",
        RejectReason::SyntheticSend => "Synthetic send failure",
        RejectReason::SyntheticFriends => "Synthetic friends failure",
        RejectReason::SyntheticSignIn => "Synthetic sign-in is unavailable",
        RejectReason::ReadFailed => "read failed",
        RejectReason::SyntheticUnsend => "Synthetic unsend rejection",
        RejectReason::NotAccepted => "Message was not accepted",
    }
}

pub(crate) fn viewer_failure(failure: Failure) -> String {
    format!("media failure: {failure:?}")
}

pub(crate) fn create_failure(failure: &line_client::engine::CreateFailure) -> &str {
    use line_client::engine::CreateFailure;
    match failure {
        CreateFailure::InvalidSelection => "Select at least two friends to create a group.",
        CreateFailure::Local => "Couldn't create group. Try again.",
        CreateFailure::Rejected => "Couldn't create group. Try again.",
        CreateFailure::Uncertain => {
            "The group may have been created. Check your conversations before trying again."
        }
    }
}

pub(crate) fn profile_failure(failure: line_client::engine::ProfileFailure) -> &'static str {
    use line_client::engine::ProfileFailure;
    match failure {
        ProfileFailure::Rename => "Couldn't rename.",
        ProfileFailure::AddFriend => "Couldn't add friend.",
        ProfileFailure::Mute => "Couldn't change notifications.",
        ProfileFailure::Leave => "Couldn't leave group.",
    }
}

pub(crate) fn pairing_stage(stage: line_client::client::pairing::Stage) -> String {
    use line_client::client::pairing::Stage;
    match stage {
        Stage::WaitingKeyring => "Waiting for the keyring to unlock…".into(),
        Stage::RecoveringKeys => "Recovering E2EE keys from saved pairing metadata".into(),
        Stage::Authenticated => {
            "SUCCESS: saved session authenticated and E2EE keys restored".into()
        }
        Stage::CreatingSession => "Creating Chrome QR pairing session".into(),
        Stage::AwaitScan => "QR ready; waiting for phone scan".into(),
        Stage::CheckingCertificate => "QR scanned; checking device certificate".into(),
        Stage::AwaitPin => "Waiting for phone PIN verification".into(),
        Stage::RequestCredentials => "Phone approved; requesting session credentials".into(),
        Stage::UnwrappingKeys => {
            "Session credentials received; unwrapping Letter Sealing keychain in WASM".into()
        }
        Stage::KeysVerified => "Letter Sealing keys unwrapped and export/import verified".into(),
        Stage::CheckingRecovery => {
            "Authenticated getProfile succeeded; checking restart recovery".into()
        }
        Stage::Synthetic => "Synthetic sign-in".into(),
        Stage::RestoredKeys(count) => {
            format!("Restored {count} Letter Sealing key(s) in a fresh WASM instance")
        }
    }
}

pub fn storage_failure(reason: line_client::engine::StorageFailure) -> &'static str {
    use line_client::engine::StorageFailure::*;
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
pub fn sign_in_failure(reason: &line_client::client::error::SignInFailure) -> String {
    use line_client::client::error::SignInFailure::*;
    match reason {
        CryptoModule(path) => format!("Crypto module missing or unsupported. Install the supported module at {} and try again.", path.display()),
        StateDirectory(path) => format!("State directory is not writable: {}. Grant write permission and try again.", path.display()),
        Keyring => "Unlock your keyring (KWallet/GNOME Keyring) and try again. Ensure Secret Service is running with a default keyring.".into(),
        Network => "Can't reach LINE. Check your network connection and try again.".into(),
        Server => "LINE refused sign-in. Try again later.".into(),
        Phone => "Phone approval was rejected or timed out. Start again and approve sign-in on your phone.".into(),
        Cancelled => String::new(),
        SavedSession(reason) => format!("Your saved sign-in can't be used: {reason}. Sign in again to replace it."),
        Expired => "Sign-in expired. Sign in again.".into(),
        SessionIdentity => "The saved sign-in belongs to a different account. Sign in again.".into(),
        Internal => "Sign-in stopped because of an internal error. See the log and try again.".into(),
    }
}

//! User-facing wording for typed library outcomes.
use line_wasm_poc::client::{desktop::assets::Failure, error::RejectReason};

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

pub(crate) fn create_failure(failure: &line_wasm_poc::engine::CreateFailure) -> &str {
    use line_wasm_poc::engine::CreateFailure;
    match failure {
        CreateFailure::InvalidSelection => "invalid store operation: Select at least two friends.",
        CreateFailure::Local(message) => message,
        CreateFailure::Rejected => "Couldn't create group.",
        CreateFailure::Uncertain => "The group may have been created.",
    }
}

pub(crate) fn profile_failure(failure: line_wasm_poc::engine::ProfileFailure) -> &'static str {
    use line_wasm_poc::engine::ProfileFailure;
    match failure {
        ProfileFailure::Rename => "Couldn't rename.",
        ProfileFailure::AddFriend => "Couldn't add friend.",
        ProfileFailure::Mute => "Couldn't change notifications.",
        ProfileFailure::Leave => "Couldn't leave group.",
    }
}

pub(crate) fn pairing_stage(stage: line_wasm_poc::client::pairing::Stage) -> String {
    use line_wasm_poc::client::pairing::Stage;
    match stage {
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

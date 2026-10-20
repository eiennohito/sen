//! LINE client protocol, crypto host and desktop application core.
pub mod attachments;
pub mod backend;
pub mod client;
pub mod controller;
pub mod credentials;
pub mod crypto_module;
pub mod engine;
pub mod images;
pub mod locations;
pub mod reporting;
pub mod runtime;
pub mod store;
pub mod system_events;
#[cfg(test)]
mod test_support;

pub(crate) fn fill_random(bytes: &mut [u8]) -> std::io::Result<()> {
    let mut remaining = bytes;
    while !remaining.is_empty() {
        // getrandom may return a short fill or be interrupted by a signal.
        let filled = unsafe { libc::getrandom(remaining.as_mut_ptr().cast(), remaining.len(), 0) };
        if filled < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if filled == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        remaining = &mut remaining[filled as usize..];
    }
    Ok(())
}

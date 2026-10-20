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
#[cfg(any(test, feature = "demo"))]
pub mod test_support;

pub(crate) fn fill_random(bytes: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(bytes)
}

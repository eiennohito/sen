//! Tests require the installed public extension module.
pub(crate) fn installed_module() -> crate::crypto_module::CryptoModule {
    crate::crypto_module::resolve(None, None).unwrap_or_else(|error| {
        panic!("WASM tests require <config>/{{google-chrome*,chromium}}/<profile>/Extensions/{}/<version>/static/js/ltsm.wasm: {error}", crate::crypto_module::EXTENSION_ID)
    })
}

//! Tests require the installed public extension module.
#[cfg(test)]
pub(crate) fn installed_module() -> crate::crypto_module::CryptoModule {
    crate::crypto_module::resolve(None, None).unwrap_or_else(|error| {
        panic!("WASM tests require <config>/{{google-chrome*,chromium}}/<profile>/Extensions/{}/<version>/static/js/ltsm.wasm: {error}", crate::crypto_module::EXTENSION_ID)
    })
}

#[cfg(test)]
pub(crate) fn compiled_module() -> &'static wasmi::Module {
    static MODULE: std::sync::OnceLock<wasmi::Module> = std::sync::OnceLock::new();
    MODULE.get_or_init(|| {
        let bytes = installed_module().read().unwrap();
        wasmi::Module::new(&wasmi::Engine::default(), &bytes).unwrap()
    })
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestRuntime(pub Option<crate::runtime::Runtime>);
#[cfg(test)]
impl From<crate::runtime::Runtime> for TestRuntime {
    fn from(runtime: crate::runtime::Runtime) -> Self {
        Self(Some(runtime))
    }
}
#[cfg(test)]
impl std::ops::Deref for TestRuntime {
    type Target = crate::runtime::Runtime;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("HTTP-only test used crypto")
    }
}
#[cfg(test)]
impl std::ops::DerefMut for TestRuntime {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().expect("HTTP-only test used crypto")
    }
}

/// Tiny JPEG with an EXIF orientation-6 IFD, constructed rather than checked in.
#[cfg(test)]
pub(crate) fn oriented_jpeg() -> Vec<u8> {
    let image = image::RgbImage::from_fn(3, 2, |x, _| {
        if x == 0 {
            image::Rgb([255, 0, 0])
        } else {
            image::Rgb([0, 0, 255])
        }
    });
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100)
        .encode_image(&image)
        .unwrap();
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut result = jpeg[..2].to_vec();
    result.extend_from_slice(&[0xff, 0xe1]);
    result.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    result.extend_from_slice(exif);
    result.extend_from_slice(&jpeg[2..]);
    result
}

/// A test directory removed when its owner drops.
pub struct TempDir(pub std::path::PathBuf);
impl TempDir {
    #[cfg(test)]
    pub(crate) fn old_account(&self, mid: &str) -> std::path::PathBuf {
        use sha2::{Digest, Sha256};
        self.0
            .join("line-desktop/accounts")
            .join(format!("{:x}", Sha256::digest(mid)))
    }
    #[cfg(test)]
    pub(crate) fn locations(&self) -> crate::locations::Locations {
        crate::locations::Locations::account(self.0.join("sen"), self.0.join("cache"))
    }
    #[cfg(test)]
    pub(crate) fn write(&self, path: &std::path::Path, value: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value).unwrap();
    }
    pub fn new(name: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/tmp")
            .join(format!(
                "{name}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

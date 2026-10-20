//! Tests require the installed public extension module.
pub(crate) fn installed_module() -> crate::crypto_module::CryptoModule {
    crate::crypto_module::resolve(None, None).unwrap_or_else(|error| {
        panic!("WASM tests require <config>/{{google-chrome*,chromium}}/<profile>/Extensions/{}/<version>/static/js/ltsm.wasm: {error}", crate::crypto_module::EXTENSION_ID)
    })
}

pub(crate) fn compiled_module() -> &'static wasmi::Module {
    static MODULE: std::sync::OnceLock<wasmi::Module> = std::sync::OnceLock::new();
    MODULE.get_or_init(|| {
        let bytes = installed_module().read().unwrap();
        wasmi::Module::new(&wasmi::Engine::default(), &bytes).unwrap()
    })
}

#[derive(Default)]
pub(crate) struct TestRuntime(pub Option<crate::runtime::Runtime>);
impl From<crate::runtime::Runtime> for TestRuntime {
    fn from(runtime: crate::runtime::Runtime) -> Self {
        Self(Some(runtime))
    }
}
impl std::ops::Deref for TestRuntime {
    type Target = crate::runtime::Runtime;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("HTTP-only test used crypto")
    }
}
impl std::ops::DerefMut for TestRuntime {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().expect("HTTP-only test used crypto")
    }
}

/// Tiny JPEG with an EXIF orientation-6 IFD, constructed rather than checked in.
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

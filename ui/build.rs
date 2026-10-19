fn main() {
    println!("cargo:rerun-if-changed=../app/ui");
    println!("cargo:rerun-if-changed=../app/assets");
    slint_build::compile_with_config(
        "../app/ui/app.slint",
        slint_build::CompilerConfiguration::new()
            .with_style("fluent".into())
            .with_debug_info(
                std::env::var_os("CARGO_FEATURE_UI_DEBUG").is_some()
                    || std::env::var_os("CARGO_FEATURE_TESTING").is_some(),
            ),
    )
    .expect("compile desktop UI");
}

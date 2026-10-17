fn main() {
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new()
            .with_style("fluent".into())
            .with_debug_info(std::env::var_os("CARGO_FEATURE_UI_DEBUG").is_some()),
    )
    .expect("compile desktop UI");
}

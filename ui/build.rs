use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=../app/ui");
    println!("cargo:rerun-if-changed=../app/assets");
    let demo = if std::env::var_os("CARGO_FEATURE_DEMO").is_some() {
        PathBuf::from("../app/ui/dev")
    } else {
        PathBuf::from("../app/ui/product")
    };
    slint_build::compile_with_config(
        "../app/ui/app.slint",
        slint_build::CompilerConfiguration::new()
            .with_library_paths(std::collections::HashMap::from([("demo".into(), demo)]))
            .with_style("fluent".into())
            .with_debug_info(
                std::env::var_os("CARGO_FEATURE_UI_DEBUG").is_some()
                    || std::env::var_os("CARGO_FEATURE_TESTING").is_some(),
            ),
    )
    .expect("compile desktop UI");
}

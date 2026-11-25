use slint_build::{CompilerConfiguration, DefaultTranslationContext};

fn main() {
    let config = CompilerConfiguration::new()
        // translations/<lang>/LC_MESSAGES/bd-app.po, compiled into the binary.
        .with_bundled_translations("translations")
        .with_default_translation_context(DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).expect("failed to compile the Slint UI");

    // Executable icon shown by Explorer and the taskbar.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=../../assets/icon.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icon.ico");
        res.set("FileDescription", "BDownloader");
        res.set("ProductName", "BDownloader");
        if let Err(e) = res.compile() {
            println!("cargo:warning=cannot embed the icon: {e}");
        }
    }
}

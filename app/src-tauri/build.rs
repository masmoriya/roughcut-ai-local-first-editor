fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // whisper.cpp's Metal availability checks need Clang's runtime.
        // rustc links with -nodefaultlibs, which omits that archive.
        let output = std::process::Command::new("xcrun")
            .args(["clang", "-print-file-name=libclang_rt.osx.a"])
            .output()
            .expect("Locate the macOS Clang runtime");
        let archive = String::from_utf8(output.stdout).expect("Clang runtime path is UTF-8");
        let archive = archive.trim();
        assert!(
            output.status.success() && std::path::Path::new(archive).is_file(),
            "The macOS Clang runtime archive is required for Whisper Metal"
        );
        println!("cargo:rustc-link-arg={archive}");
    }
    tauri_build::build();
}

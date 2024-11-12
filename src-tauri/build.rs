fn main() {
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-lib=X11");

    tauri_build::build()
}

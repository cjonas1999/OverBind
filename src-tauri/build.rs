fn main() {
    println!("cargo:rustc-link-lib=X11");
    tauri_build::build()
}

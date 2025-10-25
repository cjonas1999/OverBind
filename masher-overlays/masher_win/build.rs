use std::env;
use std::path::Path;

fn main() {
    let def_path = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("d3d11.def");
    println!("cargo:rustc-link-arg=/DEF:{}", def_path.display());
}

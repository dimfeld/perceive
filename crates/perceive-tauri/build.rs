fn set_dev_rpath() {
    println!("cargo:rustc-link-arg=-Wl,-rpath,../../torch/lib");
}

fn main() {
    #[cfg(feature = "dev-command")]
    set_dev_rpath();
    tauri_build::build()
}

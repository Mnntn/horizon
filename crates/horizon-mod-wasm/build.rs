fn main() {
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=../horizon-mod-api/src");
    println!("cargo:rerun-if-changed=../horizon-mod-api-gen/src");
    let code = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || horizon_mod_api_gen::generate_wire_host(&root))
        .expect("starting host generator")
        .join()
        .expect("host generator panicked")
        .expect("generating host boundary");
    horizon_mod_api_gen::write_if_changed(out.join("wire_host_generated.rs"), code)
        .expect("writing host boundary");
}

//! Embeds Horizon's icon (`packaging/icons/horizon.ico`) in the Windows executables, as icon
//! resource 1: Explorer and the task bar show it, and the window takes it as its icon.
fn main() {
    println!("cargo:rerun-if-env-changed=HORIZON_LAUNCHER_REPOSITORY");
    println!("cargo:rustc-check-cfg=cfg(horizon_fork_launcher)");
    let repository =
        std::env::var("HORIZON_LAUNCHER_REPOSITORY").unwrap_or_else(|_| "arismoko/horizon".into());
    match repository.as_str() {
        "arismoko/horizon" => println!("cargo:rustc-cfg=horizon_fork_launcher"),
        "horizon-rust/horizon" => {}
        _ => panic!("unsupported launcher repository: {repository}"),
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // Resolve imports before main from System32, never a nearby injector DLL (#13108).
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let icon = if repository == "arismoko/horizon" {
        "fork/launcher.ico"
    } else {
        "horizon.ico"
    };
    let ico = format!("{dir}/../packaging/icons/{icon}");
    println!("cargo:rerun-if-changed={ico}");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(&ico);
        if repository == "arismoko/horizon" {
            resource.set("FileDescription", "Horizon Fork Launcher (arismoko)");
            resource.set("ProductName", "Horizon Fork Launcher (arismoko)");
            resource.set("InternalName", "horizon-fork-launcher");
            resource.set("OriginalFilename", "horizon-fork-launcher.exe");
        }
        if let Err(e) = resource.compile() {
            panic!("embedding {ico}: {e}");
        }
    }
}

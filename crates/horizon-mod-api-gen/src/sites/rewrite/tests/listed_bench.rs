use super::*;

/// Author benchmark: execute the generated listed path, not a hand-written flag surrogate.
#[test]
#[ignore]
fn unsubscribed_listed_call_cost() {
    let source = "#[inline(never)] pub fn run(s: &str) -> usize { size(s) } fn size(s: &str) -> usize { s.len() }";
    let entries = vec![
        internal(
            "horizon_client::game::run",
            "fn",
            "fn run(s: &str) -> usize",
        ),
        internal(
            "horizon_client::game::size",
            "fn",
            "fn size(s: &str) -> usize",
        ),
    ];
    let request = Declaration {
        target: "horizon_client::game::run".into(),
        ..declaration("size", Some(0))
    };
    let (generated, sites) = rewrite(source, "horizon_client::game", &entries, &[request]).unwrap();
    let name = &sites[0].name;
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/listed-cost");
    fs::create_dir_all(directory.join("src")).unwrap();
    let macros = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../horizon-mod-api-macros");
    fs::write(directory.join("Cargo.toml"), format!("[package]\nname='listed-cost'\nversion='0.0.0'\nedition='2024'\n[workspace]\n[dependencies]\nhorizon-mod-api-macros={{path={macros:?}}}\n")).unwrap();
    fs::write(directory.join("src/main.rs"), format!(r#"
extern crate self as mod_points;
pub enum SiteId {{ {name} }}
pub mod __private {{
    pub static SITES: [std::sync::atomic::AtomicBool; 1] = [std::sync::atomic::AtomicBool::new(false)]; pub struct Table; pub static TABLE: Table = Table; impl Table {{ pub fn active_site(&self, index: usize) -> bool {{ SITES[index].load(std::sync::atomic::Ordering::Relaxed) }} }}
    pub fn wrap<O>(_: super::SiteId, _: usize, f: impl FnOnce(Option<(*const (), usize)>) -> O) -> O {{ f(std::hint::black_box(None)) }}
}}
mod listed {{ {generated} }}
mod plain {{ {source} }}
fn main() {{
    use std::hint::black_box;
    __private::SITES[0].store(black_box(false), std::sync::atomic::Ordering::Relaxed);
    const N: usize = 50_000_000;
    for round in 0..6 {{
        for (label, f) in if round % 2 == 0 {{ [("plain", plain::run as fn(&str)->usize), ("listed", listed::run as fn(&str)->usize)] }} else {{ [("listed", listed::run as fn(&str)->usize), ("plain", plain::run as fn(&str)->usize)] }} {{
            let start = std::time::Instant::now();
            for _ in 0..N {{ black_box(f(black_box("oak"))); }}
            println!("{{round}} {{label}} {{:.4}} ns/call", start.elapsed().as_nanos() as f64 / N as f64);
        }}
    }}
}}
"#)).unwrap();
    let result = std::process::Command::new("cargo")
        .args([
            "run",
            "--release",
            "--offline",
            "--quiet",
            "--manifest-path",
        ])
        .arg(directory.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", directory.join("target"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    println!("{}", String::from_utf8_lossy(&result.stdout));
    fs::remove_dir_all(directory).unwrap();
}

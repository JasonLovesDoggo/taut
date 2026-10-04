use std::fs;
use std::path::Path;

fn main() {
    let worker = fs::read_to_string("src/worker.py").expect("Failed to read src/worker.py");
    let fixtures = fs::read("src/fixtures.py").expect("Failed to read src/fixtures.py");
    // Keep parsing and imports of fixture machinery off the ordinary test path.
    let fixture_hex: String = fixtures.iter().map(|byte| format!("{byte:02x}")).collect();
    let script = format!("_FIXTURES_SOURCE_HEX = \"{fixture_hex}\"\n{worker}");
    let output_path = Path::new(&std::env::var("OUT_DIR").unwrap()).join("worker_script.rs");
    fs::write(
        output_path,
        format!("const WORKER_SCRIPT: &str = {script:?};"),
    )
    .expect("Failed to write worker_script.rs");
    println!("cargo:rerun-if-changed=src/worker.py");
    println!("cargo:rerun-if-changed=src/fixtures.py");
}

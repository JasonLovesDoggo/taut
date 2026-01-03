use std::fs;
use std::path::Path;

fn main() {
    let worker_py = fs::read_to_string("src/worker.py").expect("Failed to read src/worker.py");

    let output_path = Path::new(&std::env::var("OUT_DIR").unwrap()).join("worker_script.rs");
    let output = format!("const WORKER_SCRIPT: &str = r#\"{}\"#;", worker_py);

    fs::write(&output_path, output).expect("Failed to write worker_script.rs");

    println!("cargo:rerun-if-changed=src/worker.py");
}

// Compile the production video module, including its real surface ownership,
// worker, timing and decoder adapter. Only the SDK functions are host fakes.
use std::{env, fs, path::PathBuf};
fn main() {
    let here = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let source = here.join("../../src/streaming/video").canonicalize().unwrap();
    let mut module = String::new();
    for line in fs::read_to_string(source.join("mod.rs")).unwrap().lines() {
        if let Some(name) = line.strip_prefix("mod ").or_else(|| line.strip_prefix("pub(crate) mod ")).and_then(|s| s.strip_suffix(';')) {
            module.push_str(&format!("#[path = {:?}]\n", source.join(format!("{name}.rs"))));
        }
        module.push_str(line);
        module.push('\n');
    }
    module.push_str(&format!("#[cfg(test)]\n#[path = {:?}]\nmod pump_tests;\n", here.join("src/cases.rs")));
    fs::write(PathBuf::from(env::var("OUT_DIR").unwrap()).join("video.rs"), module).unwrap();
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed=src/cases.rs");
    println!("cargo:rerun-if-changed=src/burst_replay.rs");
}

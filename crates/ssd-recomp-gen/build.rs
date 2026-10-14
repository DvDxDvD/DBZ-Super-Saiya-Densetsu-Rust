//! Picks up the recompiled code: `.work/recomp/generated.rs` (written by
//! `cargo run --release -p xtask -- recompile`, or the file `SSD_RECOMP_FILE` names). The
//! generated source is derived from the ROM and never tracked; without it this crate is an
//! empty table and the game runs as without recompilation.

use std::path::PathBuf;
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-env-changed=SSD_RECOMP_FILE");
    println!("cargo:rerun-if-changed=build.rs");
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let path = env::var_os("SSD_RECOMP_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("../../.work/recomp/generated.rs"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("gen.rs");
    println!("cargo:rerun-if-changed={}", path.display());
    let text = if path.is_file() {
        let absolute = fs::canonicalize(&path).expect("canonical path of the generated file");
        let literal = absolute.display().to_string().replace('\\', "/");
        let literal = literal.strip_prefix("//?/").unwrap_or(&literal).to_owned();
        format!("pub const AVAILABLE: bool = true;\ninclude!(\"{literal}\");\n")
    } else {
        "pub const AVAILABLE: bool = false;\npub static TABLE: &[ssd_recomp::rt::GenEntry] = &[];\npub static MID_TABLE: &[ssd_recomp::rt::GenEntry] = &[];\npub static VARIANTS: &[ssd_recomp::rt::Variant] = &[];\npub static COV: [std::sync::atomic::AtomicU8; 0] = [];\n"
            .to_owned()
    };
    fs::write(out, text).expect("writing gen.rs");
}

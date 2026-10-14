//! Sets `SSD_CORE_VERSION`, the emulator core version that replays carry (`ssd_oracle::CORE_VERSION`):
//! `jgenesis <upstream commit>+<CRC32 of the vendored sources>`, see `build/core_version.rs`.
//!
//! The version changes with any edit of the vendored `.rs` files and crate manifests, which is
//! the code the emulator runs, and not with documentation. A new upstream version changes the
//! commit. Either costs a regeneration of the replays, never a replay played on a core that
//! runs it differently. Cargo reruns this script when anything under the vendored root changes:
//! the root and every directory under it are named, since a file deleted from a nested directory
//! changes only that directory's modification time on NTFS.

#[path = "build/core_version.rs"]
mod core_version;

use std::path::Path;

/// The vendored jgenesis, from the crate root.
const VENDORED: &str = "../../third_party/jgenesis";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build/core_version.rs");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(VENDORED);
    match core_version::directories(&root) {
        Ok(directories) => {
            for directory in directories {
                println!("cargo:rerun-if-changed={}", directory.display());
            }
        }
        Err(error) => panic!("reading {}: {error}", root.display()),
    }
    match core_version::core_version(&root) {
        Ok(version) => println!("cargo:rustc-env=SSD_CORE_VERSION={version}"),
        Err(error) => panic!("{error}"),
    }
}

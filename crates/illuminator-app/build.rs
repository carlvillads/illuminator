//! build.rs
//!

fn main() {
    println!("cargo:rerun-if-changed=packaging/windows");
    embed_resource::compile("packaging/windows/illuminator.rc", embed_resource::NONE)
        .manifest_required()
        .unwrap();
}

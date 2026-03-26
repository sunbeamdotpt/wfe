use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_dir = PathBuf::from("vendor/containerd/api");

    // Collect all .proto files, excluding internal runtime shim protos
    let proto_files: Vec<PathBuf> = walkdir(&api_dir)?
        .into_iter()
        .filter(|p| {
            let s = p.to_string_lossy();
            !s.contains("/runtime/task/") && !s.contains("/runtime/sandbox/")
        })
        .collect();

    println!(
        "cargo:warning=Compiling {} containerd proto files",
        proto_files.len()
    );

    let _out_dir = PathBuf::from(std::env::var("OUT_DIR")?);

    // Use tonic-prost-build (the tonic 0.14 way)
    let mut prost_config = prost_build::Config::new();
    prost_config.include_file("mod.rs");

    tonic_prost_build::configure()
        .build_server(false)
        .compile_with_config(
            prost_config,
            &proto_files,
            &[api_dir, PathBuf::from("proto")],
        )?;

    Ok(())
}

/// Recursively collect all .proto files under a directory.
fn walkdir(dir: &PathBuf) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut protos = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            protos.extend(walkdir(&path)?);
        } else if path.extension().is_some_and(|ext| ext == "proto") {
            protos.push(path);
        }
    }
    Ok(protos)
}

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_files = vec!["proto/wfe/v1/wfe.proto"];

    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let descriptor_path = out_dir.join("wfe_descriptor.bin");

    let mut prost_config = prost_build::Config::new();
    prost_config.include_file("mod.rs");

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .file_descriptor_set_path(&descriptor_path)
        .compile_with_config(
            prost_config,
            &proto_files,
            &["proto"],
        )?;

    Ok(())
}

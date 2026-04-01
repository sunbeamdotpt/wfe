fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_files = vec!["proto/wfe/v1/wfe.proto"];

    let mut prost_config = prost_build::Config::new();
    prost_config.include_file("mod.rs");

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_with_config(
            prost_config,
            &proto_files,
            &["proto"],
        )?;

    Ok(())
}

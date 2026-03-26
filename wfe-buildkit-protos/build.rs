use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use Go-style import paths so protoc sees each file only once
    let proto_dir = PathBuf::from("proto");
    let go_prefix = "github.com/moby/buildkit";

    let proto_files: Vec<PathBuf> = vec![
        // Core control service (Solve, Status, ListWorkers, etc.)
        "api/services/control/control.proto",
        // Types
        "api/types/worker.proto",
        // Solver / LLB definitions
        "solver/pb/ops.proto",
        // Source policy
        "sourcepolicy/pb/policy.proto",
        // Session protocols
        "session/auth/auth.proto",
        "session/filesync/filesync.proto",
        "session/secrets/secrets.proto",
        "session/sshforward/ssh.proto",
        "session/upload/upload.proto",
        "session/exporter/exporter.proto",
        // Utilities
        "util/apicaps/pb/caps.proto",
        "util/stack/stack.proto",
    ]
    .into_iter()
    .map(|p| proto_dir.join(go_prefix).join(p))
    .collect();

    println!(
        "cargo:warning=Compiling {} buildkit proto files",
        proto_files.len()
    );

    let mut prost_config = prost_build::Config::new();
    prost_config.include_file("mod.rs");

    tonic_prost_build::configure()
        .build_server(false)
        .compile_with_config(
            prost_config,
            &proto_files,
            // Include paths for import resolution:
            // 1. The vendor dir inside buildkit (for Go-style github.com/... imports)
            // 2. The buildkit root itself (for relative imports)
            // 3. Our proto/ dir (for google/rpc/status.proto)
            &[
                // proto/ has symlinks that resolve Go-style github.com/... imports
                PathBuf::from("proto"),
            ],
        )?;

    Ok(())
}

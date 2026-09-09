use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let descriptor = PathBuf::from(std::env::var("OUT_DIR")?).join("schemaforge_descriptor.bin");

    tonic_prost_build::configure()
        // Written out so the reflection service can serve the contract, which
        // is what lets grpcurl and the frontend codegen discover it.
        .file_descriptor_set_path(&descriptor)
        .compile_protos(
            &[
                "proto/schemaforge/v1/auth.proto",
                "proto/schemaforge/v1/schema.proto",
                "proto/schemaforge/v1/health.proto",
            ],
            &["proto"],
        )?;

    println!("cargo:rerun-if-changed=proto");

    Ok(())
}

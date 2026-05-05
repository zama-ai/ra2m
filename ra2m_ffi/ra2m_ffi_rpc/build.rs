fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check if protoc is available
    if std::process::Command::new("protoc")
        .arg("--version")
        .output()
        .is_err()
    {
        println!(
            "cargo:warning=protobuf-compiler (i.e. protoc) not found skipped ra2m_ffi_rpc build"
        );
        return Ok(());
    }
    tonic_build::compile_protos("src/remote_port.proto")?;
    Ok(())
}

fn main() {
    println!("cargo:rerun-if-changed=proto/abutown.proto");
    println!("cargo:rerun-if-changed=proto/traffic.proto");
    prost_build::compile_protos(&["proto/abutown.proto", "proto/traffic.proto"], &["proto/"])
        .expect("prost-build failed; verify `protoc` is in PATH (libprotoc 3.20+)");
}

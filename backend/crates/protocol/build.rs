fn main() {
    println!("cargo:rerun-if-changed=proto/abutown.proto");
    prost_build::compile_protos(&["proto/abutown.proto"], &["proto/"])
        .expect("prost-build failed; verify `protoc` is in PATH (libprotoc 3.20+)");
}

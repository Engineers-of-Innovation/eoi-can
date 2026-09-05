fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc");
    std::env::set_var("PROTOC", protoc);
    let proto = "../proto/eoi/telemetry/v1/telemetry.proto";
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&[proto], &["../proto"])
        .expect("compile telemetry.proto");
}

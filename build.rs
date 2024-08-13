use std::path::Path;

fn main() {
    let proto_dir = Path::new("src/protocol/connect/protos");

    protobuf_codegen::Codegen::new()
        .protoc()
        .protoc_path(&protoc_bin_vendored::protoc_bin_path().expect("could not find protoc binary"))
        .cargo_out_dir("protos")
        .include(proto_dir)
        .input(proto_dir.join("queue.proto"))
        .input(proto_dir.join("repeat.proto"))
        .run_from_script();
}

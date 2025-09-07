//! Build script for Pleezer.
//!
//! This script:
//! 1. Sets Git-related environment variables if available:
//!    * `PLEEZER_COMMIT_HASH` - Abbreviated commit hash
//!    * `PLEEZER_COMMIT_DATE` - Commit date
//! 2. Generates Rust code from Protocol Buffer definitions in `src/protocol/connect/protos/`
//!
//! The Git information can be accessed at runtime using:
//! * `env!("PLEEZER_COMMIT_HASH")` for the commit hash
//! * `env!("PLEEZER_COMMIT_DATE")` for the commit date

use std::path::Path;

use git2::Repository;
use time::OffsetDateTime;

fn main() {
    if let Ok(repo) = Repository::open(".")
        && let Some(commit) = repo.head().ok().and_then(|head| head.peel_to_commit().ok())
    {
        if let Some(hash) = commit
            .as_object()
            .short_id()
            .ok()
            .and_then(|buf| buf.as_str().map(|s| s.to_string()))
        {
            println!("cargo:rustc-env=PLEEZER_COMMIT_HASH={hash}");
        }

        if let Ok(timestamp) = OffsetDateTime::from_unix_timestamp(commit.time().seconds()) {
            let format = time::format_description::parse("[year]-[month]-[day]")
                .expect("invalid date format string");
            println!(
                "cargo:rustc-env=PLEEZER_COMMIT_DATE={}",
                timestamp.format(&format).expect("could not format date")
            );
        }
    }

    let proto_dir = Path::new("src/protocol/connect/protos");
    protobuf_codegen::Codegen::new()
        .pure()
        .cargo_out_dir("protos")
        .include(proto_dir)
        .input(proto_dir.join("queue.proto"))
        .input(proto_dir.join("repeat.proto"))
        .run_from_script();
}

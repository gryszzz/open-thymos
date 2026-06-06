//! Embeds the current git short SHA so `thymos --version` can report exactly
//! which build is running (dev vs. release, and the precise commit) — the
//! "track changes" signal. Falls back to "unknown" outside a git checkout
//! (e.g. a source tarball), so the build never fails for lack of git.
use std::process::Command;

fn main() {
    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=THYMOS_GIT_SHA={sha}");

    // Best-effort: rebuild when HEAD moves so the SHA stays fresh in dev. The
    // path is relative to this crate dir (thymos/crates/thymos-cli). Only emit
    // it when present to avoid a warning in tarball builds with no .git.
    for rel in ["../../../.git/HEAD", "../../../.git/packed-refs"] {
        if std::path::Path::new(rel).exists() {
            println!("cargo:rerun-if-changed={rel}");
        }
    }
}

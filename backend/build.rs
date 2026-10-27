use std::{path::Path, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for name in ["HEAD", "refs", "packed-refs"] {
        if let Some(path) =
            git(&["rev-parse", "--git-path", name]).filter(|path| Path::new(path).exists())
        {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let hash = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=ZJUDSS_GIT_HASH={hash}");
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

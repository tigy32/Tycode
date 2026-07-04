//! The `--version` flag is a public probe contract: consumers (e.g. Tyde
//! setup) verify installed binaries with it, so it must print and exit
//! without starting the actor or waiting on stdin.

use std::process::Command;

fn expect_version_output(flag: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_tycode-subprocess"))
        .arg(flag)
        .output()
        .expect("the binary must run and exit on its own");
    assert!(output.status.success(), "{flag} must exit successfully");
    let stdout = String::from_utf8(output.stdout).expect("version output is utf-8");
    assert_eq!(
        stdout.trim(),
        format!("tycode-subprocess {}", env!("CARGO_PKG_VERSION")),
        "version output must be '<binary> <version>'"
    );
}

#[test]
fn version_flag_prints_and_exits() {
    expect_version_output("--version");
}

#[test]
fn short_version_flag_prints_and_exits() {
    expect_version_output("-V");
}

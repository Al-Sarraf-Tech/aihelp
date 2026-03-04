use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;

#[test]
fn help_includes_setup_manpage_sections() {
    cargo_bin_cmd!("aihelp")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("MANPAGE"))
        .stdout(contains("aihelp --setup"))
        .stdout(contains("STREAMING"))
        .stdout(contains("--no-stream"))
        .stdout(contains("RETRIES"))
        .stdout(contains("--retries"))
        .stdout(contains("MODEL WORKFLOW"))
        .stdout(contains("MCP WORKFLOW"));
}

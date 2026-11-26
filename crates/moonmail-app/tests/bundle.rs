use std::{fs, process::Command};

#[test]
fn packaging_can_run_while_the_mail_archive_is_unavailable() -> std::io::Result<()> {
    let dir = tempfile::tempdir()?;
    let unavailable = dir.path().join("unavailable");
    fs::write(&unavailable, b"this is a file")?;
    let output = Command::new(env!("CARGO_BIN_EXE_moonmail"))
        .args(["bundle", "--help"])
        .env("MOONMAIL_DATA_DIR", unavailable.join("archive"))
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("bundle [--dist DIR]"));
    Ok(())
}

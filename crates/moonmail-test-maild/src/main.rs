use {
    moonmail_test_maild::{Config, Error, Result, Server},
    std::{path::PathBuf, time::Duration},
};

fn run() -> Result {
    let mut config = Config {
        ports: [1993, 1465, 1143, 1587],
        ..Default::default()
    };
    let mut state = PathBuf::from("target/test-maild");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bind" => {
                config.bind = args
                    .next()
                    .ok_or(Error::Invalid("Missing bind address"))?
                    .parse()
                    .map_err(|_| Error::Invalid("Invalid bind address"))?
            },
            "--state" => {
                state = args
                    .next()
                    .ok_or(Error::Invalid("Missing state directory"))?
                    .into()
            },
            "--ephemeral-ports" => config.ports = [0; 4],
            "--server-sent" => config.server_sent = true,
            "--help" => {
                println!(
                    "moonmail-test-maild [--bind IP] [--state DIR] [--ephemeral-ports] [--server-sent]\nEphemeral fixture accounts: alice@moonmail.test / bob@moonmail.test\nPassword: moonmail-test-only\nDefault ports: IMAPS 1993, SMTPS 1465, IMAP STARTTLS 1143, SMTP STARTTLS 1587\nWrites ca.der, ca.pem and ready.json; mail and TLS keys remain in memory."
                );
                return Ok(());
            },
            _ => return Err(Error::Invalid("Unknown argument; use --help")),
        }
    }
    std::fs::create_dir_all(&state)?;
    // Remove stale readiness before binding; a failed startup must not look ready.
    match std::fs::remove_file(state.join("ready.json")) {
        Ok(()) => {},
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
        Err(e) => return Err(e.into()),
    }
    let server = Server::start(config)?;
    std::fs::write(state.join("ca.der"), &server.ca_der)?;
    std::fs::write(state.join("ca.pem"), &server.ca_pem)?;
    std::fs::write(
        state.join("ready.json.tmp"),
        serde_json::to_vec(&server.endpoints)?,
    )?;
    std::fs::rename(state.join("ready.json.tmp"), state.join("ready.json"))?;
    println!(
        "Test mail daemon ready: {}",
        serde_json::to_string(&server.endpoints)?
    );
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        },
    }
}

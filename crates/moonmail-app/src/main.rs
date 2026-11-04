mod account_setup;
mod app_icon;
mod cli;
mod fonts;
mod local;
mod navigation;
mod page;
mod provider;

use {
    anyhow::Result,
    fs2::FileExt,
    moonframe::Moonframe,
    std::{fs::OpenOptions, process::ExitCode},
};

#[derive(moonframe::rust_embed::RustEmbed)]
#[crate_path = "moonframe::rust_embed"]
#[folder = "app"]
struct AppFiles;

fn run() -> Result<ExitCode> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let check = args.iter().any(|arg| arg == "--check");
    let temporary = if check {
        Some(tempfile::tempdir()?)
    } else {
        None
    };
    let root = if let Some(dir) = &temporary {
        dir.path().to_owned()
    } else {
        local::root()?
    };
    std::fs::create_dir_all(&root)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("app.lock"))?;
    lock.try_lock_exclusive().map_err(|_| {
        anyhow::anyhow!(
            "This archive is already open in Moonmail: {}",
            root.display()
        )
    })?;
    let archive = moonmail_store::Archive::open(&root)?;
    archive.recover_interrupted_sends()?;
    drop(archive);
    if cli::dispatch(&root, &args)? {
        return Ok(ExitCode::SUCCESS);
    }
    let demo = check || std::env::var_os("MOONMAIL_DEMO").is_some();
    if demo {
        moonmail_store::seed_demo(
            &mut moonmail_store::Archive::open(&root)?,
            &moonmail_store::FileBlobs::new(&root),
        )?;
    }
    // An unreadable preferences file gets the default icon here; the mail
    // provider loads the same file and reports the error in the window.
    let icon = fonts::Preferences::load(&root)
        .map(|preferences| preferences.app_icon)
        .unwrap_or_default();
    let mut app = Moonframe::new(if demo { "moonmail-demo" } else { "moonmail" })
        .title("Moonmail")
        .app_id("dev.moonmail.app")
        .version(env!("CARGO_PKG_VERSION"))
        .icon(icon.png())
        .keep(["feedback"])
        .embedded_app::<AppFiles>();
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/app");
    if cfg!(debug_assertions) {
        app = app.app_dir(source);
    }
    let font_config = args
        .windows(2)
        .find(|pair| pair[0] == "--user")
        .map(|pair| std::path::PathBuf::from(&pair[1]).join("style.luau"))
        .unwrap_or_else(|| {
            if check || args.iter().any(|arg| arg == "--no-user") {
                root.join("ui/style.luau")
            } else {
                fonts::path(demo)
            }
        });
    Ok(app
        .native("mail", move || {
            provider::Mail::new(root.clone(), demo).with_font_config(font_config.clone())
        })
        .run())
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Moonmail: {error:#}");
            ExitCode::FAILURE
        },
    }
}

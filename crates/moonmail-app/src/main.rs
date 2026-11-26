mod account_setup;
mod app_icon;
mod cli;
mod columns;
mod file_export;
mod fonts;
mod layout;
mod live_maildir;
mod local;
mod mail_import;
mod msgvault;
mod navigation;
mod page;
mod provider;
mod reader_resources;

use {
    anyhow::Result,
    fs2::FileExt,
    moonframe::{
        Moonframe,
        gpui_kit::{AssetSource, SharedString},
    },
    std::{borrow::Cow, fs::OpenOptions, process::ExitCode},
};

#[derive(moonframe::rust_embed::RustEmbed)]
#[crate_path = "moonframe::rust_embed"]
#[folder = "app"]
struct AppFiles;

#[derive(moonframe::rust_embed::RustEmbed)]
#[crate_path = "moonframe::rust_embed"]
#[folder = "assets"]
#[include = "icons/*.svg"]
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(Self::get(path).map(|file| file.data))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter(|name| name.starts_with(path))
            .map(|name| SharedString::from(name.into_owned()))
            .collect())
    }
}

fn run() -> Result<ExitCode> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let check = args.iter().any(|arg| arg == "--check")
        || matches!(args.first().map(String::as_str), Some("mockup" | "mockups"));
    let packaging = args.first().is_some_and(|arg| arg == "bundle");
    let temporary = if check || packaging {
        Some(tempfile::tempdir()?)
    } else {
        None
    };
    let root = if let Some(dir) = &temporary {
        dir.path().to_owned()
    } else {
        local::root()?
    };
    if args.first().is_some_and(|command| command == "import")
        && let Some(source) = args.get(1)
    {
        mail_import::check_destination(source.as_ref(), &root)?;
    }
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
    archive.recover_interrupted_sent_copies()?;
    archive.recover_interrupted_moves()?;
    archive.recover_interrupted_folders()?;
    drop(archive);
    let first = args.first().map(String::as_str);
    if !check
        && !packaging
        && std::env::var_os("MOONMAIL_DEMO").is_none()
        && first.is_none_or(|arg| arg == "msgvault" || arg.starts_with('-'))
        && let Some(database) = msgvault::locate()
    {
        match msgvault::detect(&root, &database) {
            Ok(added) => {
                for account in added {
                    eprintln!("Moonmail: connected {account} from {}", database.display());
                }
            },
            Err(error) => eprintln!("Moonmail: msgvault was not connected: {error:#}"),
        }
    }
    if cli::dispatch(&root, &args)? {
        return Ok(ExitCode::SUCCESS);
    }
    let demo = !packaging && (check || std::env::var_os("MOONMAIL_DEMO").is_some());
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
        .assets(AppAssets)
        .embedded_app::<AppFiles>()
        .project_dir(env!("CARGO_MANIFEST_DIR"));
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

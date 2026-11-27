//! macOS comparison of the production selection and renderer paths. Both views
//! stay warm and display the same synthetic archive, one message at a time.
use {
    super::*,
    crate::{native_reader::ReaderView, reader_backend::Renderer},
    moonframe::{
        HostView as _,
        gpui_kit::{self as kit, AppContext as _, HeadlessAppContext, base::Root, px, size},
    },
    std::{
        io::{BufRead, BufReader, Write},
        path::Path,
        process::{Command, Stdio},
        sync::Arc,
        time::Instant,
    },
};

const CASES: [&str; 3] = ["plain", "receipt", "newsletter"];
const WARM_RUNS: usize = 20;

fn fixture(
    archive: &mut Archive,
    blobs: &FileBlobs,
    case: &str,
    tracker: &str,
) -> Result<MessageId> {
    let mailbox = archive.mailbox("bench@example.org", "INBOX")?;
    let details = r#"<p><b>Studio membership</b><br>Monthly plan · $24.00</p>"#;
    let (kind, body) = match case {
        "plain" => (
            "text/plain",
            "Hello,\n\nHere are the notes from our studio meeting.\n\nThe next review is on Thursday.".to_owned(),
        ),
        "receipt" => {
            let html = format!(r##"<style>.card {{background:#fff;border:1px solid #DCDFE3;border-radius:16px;padding:24px;box-shadow:0px 0px 4px rgba(69,75,82,0.25)}} .title {{font-size:32px;font-weight:700}}</style>
        <table width="100%" bgcolor="#f8fafb"><tr><td align="center"><table width="600"><tr><td style="padding:20px">
        <img alt="Northline Studio" src="{tracker}"><p>Hello,</p><p class="title">Payment received</p>
        <div class="card"><p>Payment details</p>{details}<a style="background:#0057b8;color:#fff;border-radius:16px;padding:8px" href="{tracker}">View receipt</a></div></td></tr></table></td></tr></table>"##);
            ("text/html", html)
        }
        "newsletter" => {
            let html = (0..60).map(|n| {
                format!("<h2>Studio bulletin {n}</h2><p>Field notes and updates from the workshop.</p>{details}")
            }).collect();
            ("text/html", html)
        }
        _ => bail!("Unknown benchmark fixture {case}"),
    };
    let raw = format!(
        "From: Studio <studio@example.org>\r\nTo: Reader <bench@example.org>\r\nSubject: {case}\r\nMIME-Version: 1.0\r\nContent-Type: {kind}; charset=utf-8\r\n\r\n{body}"
    );
    Ok(archive.ingest(
        &moonmail_mime::StalwartParser::default(),
        blobs,
        moonmail_store::IncomingMessage {
            account: "bench@example.org",
            mailbox,
            origin: case,
            epoch: 0,
            uid: None,
            seen: true,
            starred: false,
            received_at: 1,
            raw: raw.as_bytes(),
        },
    )?)
}

pub(crate) fn run(output: &Path, swift: &Path) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut archive = Archive::open(temp.path())?;
    let blobs = FileBlobs::new(temp.path());
    let trap = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    trap.set_nonblocking(true)?;
    let tracker = format!("http://{}/image", trap.local_addr()?);
    let ids = CASES
        .iter()
        .map(|case| fixture(&mut archive, &blobs, case, &tracker))
        .collect::<Result<Vec<_>>>()?;
    drop(archive);
    std::fs::create_dir_all(output)?;
    let output = output.canonicalize()?;
    let mut mail = Mail::new(temp.path().to_owned(), true);
    mail.initialize()?;
    let mut cx = HeadlessAppContext::with_platform(
        kit::platform::current_platform(true).text_system(),
        Arc::new(()),
        kit::platform::current_headless_renderer,
    );
    cx.update(kit::init);
    let mut reader = None;
    let handle = cx.open_window(size(px(720.), px(480.)), |window, cx| {
        let view = cx.new(|_| ReaderView::new(mail.documents.clone(), &Data::Nil));
        reader = Some(view.clone());
        cx.new(|cx| Root::new(view, window, cx))
    })?;
    let reader = reader.context("Missing benchmark reader")?;
    let mut web = Command::new(swift)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut input = web.stdin.take().context("WebKit benchmark input")?;
    let mut replies = BufReader::new(web.stdout.take().context("WebKit benchmark output")?);
    let mut samples = Vec::new();
    for round in 0..=WARM_RUNS {
        for (n, id) in ids.iter().enumerate() {
            let case = CASES[n];
            let order = if round % 2 == 0 {
                [Renderer::Native, Renderer::Webkit]
            } else {
                [Renderer::Webkit, Renderer::Native]
            };
            for renderer in order {
                mail.preferences.reader_renderer = renderer;
                let start = Instant::now();
                mail.select(*id, true)?;
                let preparation = start.elapsed().as_micros();
                let name = match renderer {
                    Renderer::Native => "native",
                    Renderer::Webkit => "webkit",
                };
                let shot = output.join(format!("{name}-{case}.png"));
                let paint = match renderer {
                    Renderer::Webkit => {
                        let url = if mail.html_url.is_empty() {
                            &mail.plain_url
                        } else {
                            &mail.html_url
                        };
                        writeln!(
                            input,
                            "{}",
                            serde_json::to_string(&(
                                url,
                                if round == 0 {
                                    shot.to_string_lossy().into_owned()
                                } else {
                                    String::new()
                                }
                            ))?
                        )?;
                        input.flush()?;
                        let mut reply = String::new();
                        replies.read_line(&mut reply)?;
                        reply
                            .trim()
                            .parse::<u128>()
                            .context("WebKit snapshot failed")?
                    },
                    Renderer::Native => {
                        let props = Data::map([
                            ("revision", Data::Integer(mail.reader_revision as i64)),
                            ("rich", Data::Bool(true)),
                        ]);
                        let begin = Instant::now();
                        cx.update_entity(&reader, |reader, cx| {
                            reader.set_props(&props, cx);
                            cx.notify();
                        });
                        let image = loop {
                            cx.run_until_parked();
                            cx.update_window(handle.into(), |_, w, cx| {
                                w.refresh();
                                w.draw(cx).clear(cx);
                            })?;
                            let img = cx.capture_screenshot(handle.into())?;
                            let snapshot_time = begin.elapsed().as_micros();
                            // Opaque dark glyphs, not the blank shell/background.
                            if img
                                .pixels()
                                .filter(|p| p[3] > 240 && p[0] < 80 && p[1] < 80 && p[2] < 80)
                                .take(80)
                                .count()
                                == 80
                            {
                                break (img, snapshot_time);
                            }
                            anyhow::ensure!(
                                begin.elapsed().as_secs() < 5,
                                "No native text frame for {case}"
                            );
                        };
                        let (image, elapsed) = image;
                        if round == 0 {
                            image.save(&shot)?;
                        }
                        elapsed
                    },
                };
                samples.push(json!({"round":round,"message":case,"renderer":name,"prepare_us":preparation,"frame_us":paint,"total_us":preparation+paint}));
            }
        }
    }
    drop(input);
    anyhow::ensure!(web.wait()?.success(), "WebKit benchmark failed");
    anyhow::ensure!(
        matches!(trap.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock),
        "The reader requested a remote image"
    );
    std::fs::write(
        output.join("samples.json"),
        serde_json::to_vec_pretty(&samples)?,
    )?;
    for case in CASES {
        for backend in ["native", "webkit"] {
            let mut us: Vec<u64> = samples
                .iter()
                .filter(|s| s["round"] != 0 && s["message"] == case && s["renderer"] == backend)
                .filter_map(|s| s["total_us"].as_u64())
                .collect();
            us.sort_unstable();
            let n = us.len();
            anyhow::ensure!(n == WARM_RUNS, "Incomplete benchmark for {case}/{backend}");
            let median = (us[(n - 1) / 2] + us[n / 2]) as f64 / 2000.;
            let p95 = us[(n * 95).div_ceil(100) - 1] as f64 / 1000.;
            println!(
                "{case:12} {backend:7}  p50 {:7.2} ms  p95 {:7.2} ms  (n={})",
                median,
                p95,
                us.len()
            );
        }
    }
    Ok(())
}

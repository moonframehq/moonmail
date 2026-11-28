//! Fetch reader images independently of both renderers. Only re-encoded PNGs
//! enter a document. No browser session, account credentials or referrer are used.
use {
    crate::reader_resources::{EmbeddedImages, external_url, raster_png},
    anyhow::{Context, Result, ensure},
    base64::{Engine, engine::general_purpose::STANDARD},
    reqwest::{
        blocking::Client,
        dns::{Addrs, Name, Resolve, Resolving},
    },
    std::{
        io::Read,
        net::IpAddr,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    },
    url::{Host, Url},
};

pub const MAX_IMAGES: usize = 16;
const MAX_BYTES: usize = 2 * 1024 * 1024;

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..128).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..32).contains(&b))
                || (a == 192
                    && (b == 168 || (b == 0 && matches!(c, 0 | 2)) || (b == 88 && c == 99)))
                || (a == 198 && (matches!(b, 18 | 19) || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        },
        IpAddr::V6(ip) => {
            let [a, b, ..] = ip.segments();
            // Global unicast only, excluding transition, protocol and documentation space.
            (0x2000..0x4000).contains(&a)
                && a != 0x2002
                && !(a == 0x2001 && (b < 0x200 || b == 0xdb8))
                && !(a == 0x3fff && b < 0x1000)
        },
    }
}

struct PublicDns;
impl Resolve for PublicDns {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            // Pass verified addresses directly to the connector, including after redirects.
            let absolute = format!("{}.", name.as_str().trim_end_matches('.'));
            let addrs = tokio::net::lookup_host((absolute.as_str(), 0))
                .await?
                .filter(|addr| public_ip(addr.ip()))
                .collect::<Vec<_>>();
            if addrs.is_empty() {
                return Err(std::io::Error::other("No public image address").into());
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

pub fn image_url(value: &str) -> Option<String> {
    let mut url = Url::parse(&external_url(value)?).ok()?;
    if url.port().is_some() {
        return None;
    }
    match url.host()? {
        Host::Ipv4(ip) if !public_ip(ip.into()) => return None,
        Host::Ipv6(ip) if !public_ip(ip.into()) => return None,
        Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.');
            if !domain.contains('.') || domain.ends_with(".localhost") || domain.ends_with(".local")
            {
                return None;
            }
        },
        _ => {},
    }
    url.set_fragment(None);
    Some(url.into())
}

fn fetch(client: &Client, source: &str, cancelled: &AtomicBool, until: Instant) -> Result<Vec<u8>> {
    let mut url = Url::parse(source)?;
    for _ in 0..=3 {
        ensure!(!cancelled.load(Ordering::Relaxed), "Cancelled");
        let time = until.saturating_duration_since(Instant::now());
        ensure!(!time.is_zero(), "Image request expired");
        let mut response = client
            .get(url.clone())
            .timeout(time.min(Duration::from_secs(6)))
            .send()?;
        if response.status().is_redirection() {
            let next = url.join(
                response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .context("Missing image location")?
                    .to_str()?,
            )?;
            ensure!(
                image_url(next.as_str()).is_some()
                    && !(url.scheme() == "https" && next.scheme() == "http"),
                "Unsupported image redirect"
            );
            url = next;
            continue;
        }
        ensure!(
            response.status().is_success()
                && response.content_length().unwrap_or(0) <= MAX_BYTES as u64,
            "Unavailable or oversized image"
        );
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_BYTES, "Oversized image");
        return Ok(bytes);
    }
    anyhow::bail!("Too many image redirects")
}

pub fn download(urls: Vec<String>, cancelled: &AtomicBool) -> EmbeddedImages {
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .dns_resolver(Arc::new(PublicDns))
        .build();
    let until = Instant::now() + Duration::from_secs(25);
    collect(urls, cancelled, |url| {
        fetch(
            client.as_ref().ok().context("Image client unavailable")?,
            url,
            cancelled,
            until,
        )
    })
}

fn collect(
    urls: Vec<String>,
    cancelled: &AtomicBool,
    mut read: impl FnMut(&str) -> Result<Vec<u8>>,
) -> EmbeddedImages {
    let mut result = EmbeddedImages::default();
    let mut total = 0;
    for url in urls.into_iter().take(MAX_IMAGES) {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        if let Some(url) = image_url(&url)
            && let Ok(bytes) = read(&url)
            && bytes.len() <= MAX_BYTES
            && let Ok(png) = raster_png(&bytes)
            && png.len() <= MAX_BYTES
            && total + png.len() <= 8 * 1024 * 1024
        {
            total += png.len();
            result.resources.insert(
                url,
                format!("data:image/png;base64,{}", STANDARD.encode(png)),
            );
        } else {
            result.skipped += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_only_offers_public_image_sources_and_collapses_duplicates() -> Result<()> {
        for source in [
            "file:///tmp/a",
            "http://localhost/x",
            "http://a.local/x",
            "http://2130706433/x",
            "http://10.0.0.1/x",
            "http://169.254.169.254/x",
            "http://[::ffff:127.0.0.1]/x",
            "http://[fc00::1]/x",
            "https://user@example.com/x",
            "https://images.example:9443/x",
        ] {
            assert!(image_url(source).is_none());
        }
        let html = r#"<img src="https://images.example/logo#a"><img src="https://images.example/logo"><img src="cid:local"><a href="https://links.example/x">go</a><style>p{background:url(https://styles.example/x)}</style>"#;
        let found = crate::reader_resources::links("", Some(html))?;
        assert_eq!(found.image_urls, ["https://images.example/logo"]);
        assert_eq!(found.items, ["https://links.example/x"]);
        Ok(())
    }

    #[test]
    fn bounded_fetch_decodes_rasters_and_rejects_redirects_to_private_addresses() -> Result<()> {
        use std::{
            io::{BufRead, BufReader, Write},
            net::TcpListener,
        };
        let server = TcpListener::bind(("127.0.0.1", 0))?;
        let address = server.local_addr()?;
        let public = "http://images.example";
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .resolve("images.example", address)
            .build()?;
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2).write_to(&mut png, image::ImageFormat::Png)?;
        let handle = std::thread::spawn(move || -> Result<()> {
            for path in ["/image", "/redirect", "/large", "/vector"] {
                let (mut socket, _) = server.accept()?;
                let lines = BufReader::new(&socket)
                    .lines()
                    .take_while(|line| line.as_ref().is_ok_and(|line| !line.is_empty()))
                    .collect::<std::io::Result<Vec<_>>>()?;
                assert!(lines[0].starts_with(&format!("GET {path} ")));
                assert!(lines.iter().all(|line| {
                    !["cookie:", "authorization:", "referer:"]
                        .iter()
                        .any(|h| line.to_lowercase().starts_with(h))
                }));
                match path {
                    "/image" => {
                        write!(socket,"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n", png.get_ref().len())?;
                        socket.write_all(png.get_ref())?;
                    },
                    "/redirect" => socket.write_all(b"HTTP/1.1 302 Found\r\nConnection: close\r\nLocation: http://127.0.0.1/denied\r\nContent-Length: 0\r\n\r\n")?,
                    "/large" => socket.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 9999999\r\n\r\n")?,
                    _ => socket.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 6\r\n\r\n<svg/>")?,
                }
            }
            Ok(())
        });
        let urls = ["image", "redirect", "large", "vector"].map(|path| format!("{public}/{path}"));
        let cancelled = AtomicBool::new(false);
        let result = collect(urls.to_vec(), &cancelled, |url| {
            fetch(
                &client,
                url,
                &cancelled,
                Instant::now() + Duration::from_secs(3),
            )
        });
        assert_eq!(result.resources.len(), 1);
        assert!(result.resources[&urls[0]].starts_with("data:image/png;base64,"));
        assert_eq!(result.skipped, 3);
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("fixture failed"))??;
        Ok(())
    }
}

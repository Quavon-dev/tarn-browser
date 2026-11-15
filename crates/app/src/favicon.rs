//! Site icons (threat model S14). WebKit fetches them inside the tab, so content rules, proxy
//! and user agent apply; the shell only picks same-site URLs, or another site's the page already
//! loaded from, and decodes the bytes with limits.

use std::io::Cursor;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use image::{ImageFormat, ImageReader, Limits, Rgba, RgbaImage, imageops, imageops::FilterType};
use url::Url;

pub const MAX_BYTES: usize = 256 * 1024;
const MAX_DIMENSION: u32 = 1024;
const MAX_URL_LEN: usize = 2048;
const MAX_CANDIDATES: usize = 4;
const MAX_LOADED_HOSTS: usize = 64;
pub const SIZE: u32 = 64;
pub const MAX_TRIES: u8 = 3;
/// A stored icon older than this (in milliseconds) is fetched again, and kept if that fails.
pub const MAX_AGE: i64 = 30 * 24 * 3_600_000;

/// An icon may come from the page's host or a subdomain of it (a `www.` prefix is ignored), so a
/// page cannot make the browser contact an unrelated server.
fn same_site(page_host: &str, icon_host: &str) -> bool {
    let base = page_host.strip_prefix("www.").unwrap_or(page_host);
    icon_host == page_host || icon_host == base || icon_host.ends_with(&format!(".{base}"))
}

/// Icon URLs to try, best first: the largest declared icon, then `/favicon.ico`. `links` is the
/// engine's `href<TAB>sizes` lines, then an empty line and the hosts the page loaded resources
/// from; an icon on another site counts only from one of those hosts, so the page cannot make the
/// browser contact a server it has not used already (a site's own CDN). An SVG counts as `SIZE`,
/// since the engine rasterises it at that scale or above.
pub fn candidates(page: &Url, links: &str) -> Vec<Url> {
    let Some(page_host) = page.host_str().filter(|_| page.scheme() == "https") else { return Vec::new() };
    let (links, loaded) = links.split_once("\n\n").unwrap_or((links, ""));
    let loaded: Vec<&str> = loaded.lines().take(MAX_LOADED_HOSTS).collect();
    let mut found: Vec<(u32, Url)> = links
        .lines()
        .take(16)
        .filter_map(|line| {
            let (href, sizes) = line.split_once('\t').unwrap_or((line, ""));
            let mut url = Url::parse(href).ok().filter(|u| u.scheme() == "https" && href.len() <= MAX_URL_LEN)?;
            if !url.username().is_empty()
                || url.password().is_some()
                || url.port_or_known_default() != page.port_or_known_default()
                || !(same_site(page_host, url.host_str()?) || loaded.contains(&url.host_str()?))
            {
                return None;
            }
            url.set_fragment(None);
            let scalable = url.path().to_ascii_lowercase().ends_with(".svg") || sizes.eq_ignore_ascii_case("any");
            let declared = sizes.split_whitespace().filter_map(|s| s.split(['x', 'X']).next()?.parse::<u32>().ok()).max();
            let size = if scalable { SIZE } else { declared.unwrap_or(32) };
            Some((size, url))
        })
        .collect();
    found.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
    let fallback = page.join("/favicon.ico").ok();
    let mut out: Vec<Url> = Vec::new();
    for (_, url) in found {
        if out.len() + 1 < MAX_CANDIDATES && !out.contains(&url) && Some(&url) != fallback.as_ref() {
            out.push(url);
        }
    }
    out.extend(fallback);
    out
}

/// Hosts whose stored icon may stand for `host`, nearest first: the host, each parent up to its
/// registrable domain, then that domain's `www.` host. Never climbs above the registrable domain,
/// so one tenant of a shared host never shows another's icon.
pub fn lookup_hosts(host: &str) -> Vec<String> {
    let host = host.trim_end_matches('.');
    let site = engine::cookies::site(host);
    let mut hosts = vec![host.to_owned()];
    let mut rest = host;
    while rest != site
        && let Some((_, parent)) = rest.split_once('.')
    {
        hosts.push(parent.to_owned());
        rest = parent;
    }
    let www = format!("www.{site}");
    if rest == site && site.contains('.') && site.parse::<std::net::IpAddr>().is_err() && !hosts.contains(&www) {
        hosts.push(www);
    }
    hosts
}

/// Decodes base64 icon bytes within size limits and returns a `SIZE`×`SIZE` PNG, centred on a
/// transparent square when the icon is not square.
pub fn normalize(encoded: &str) -> Option<Vec<u8>> {
    if encoded.is_empty() || encoded.len() > MAX_BYTES.div_ceil(3) * 4 {
        return None;
    }
    let bytes = STANDARD.decode(encoded).ok()?;
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    if !matches!(reader.format()?, ImageFormat::Png | ImageFormat::Ico | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP) {
        return None;
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let icon = reader.decode().ok()?.to_rgba8();
    let (box_size, mut square) = if is_dark_on_clear(&icon) {
        (SIZE * 3 / 4, RgbaImage::from_pixel(SIZE, SIZE, Rgba([0xeb, 0xeb, 0xf0, 0xff])))
    } else {
        (SIZE, RgbaImage::new(SIZE, SIZE))
    };
    let fitted = image::DynamicImage::ImageRgba8(icon).resize(box_size, box_size, FilterType::Lanczos3).to_rgba8();
    imageops::overlay(&mut square, &fitted, i64::from((SIZE - fitted.width()) / 2), i64::from((SIZE - fitted.height()) / 2));
    let mut png = Vec::new();
    square.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).ok()?;
    Some(png)
}

/// A dark mark on a transparent background disappears on the dark sidebar, so it gets a light
/// plate, as Safari does.
fn is_dark_on_clear(icon: &RgbaImage) -> bool {
    let total = icon.pixels().len().max(1);
    let opaque: Vec<_> = icon.pixels().filter(|p| p[3] > 128).collect();
    if opaque.len() * 5 > total * 4 || opaque.is_empty() {
        return false;
    }
    let luma: u64 = opaque.iter().map(|p| (u64::from(p[0]) * 299 + u64::from(p[1]) * 587 + u64::from(p[2]) * 114) / 1000).sum();
    luma / opaque.len() as u64 <= 60
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(width: u32, height: u32) -> String {
        let mut png = Vec::new();
        RgbaImage::new(width, height).write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();
        STANDARD.encode(png)
    }

    #[test]
    fn candidates_prefer_large_same_site_raster_icons() {
        let page = Url::parse("https://www.a.test/docs/x").unwrap();
        let links = "https://www.a.test/i16.png\t16x16\n\
                     https://static.a.test/touch.png\t180x180\n\
                     https://tracker.test/p.png?uid=1\t512x512\n\
                     http://a.test/plain.png\t256x256\n\
                     https://a.test/logo.svg\t\n\
                     https://user@a.test/u.png\t64x64\n\
                     https://a.test:8443/p.png\t128x128\n\
                     javascript:alert(1)\t";
        let urls: Vec<String> = candidates(&page, links).into_iter().map(String::from).collect();
        assert_eq!(urls, ["https://static.a.test/touch.png", "https://a.test/logo.svg", "https://www.a.test/i16.png", "https://www.a.test/favicon.ico"]);
        assert!(candidates(&Url::parse("http://a.test/").unwrap(), "").is_empty());
        let many = "https://a.test/1.png\t96x96\nhttps://a.test/2.png\t48x48\nhttps://a.test/3.png\t16x16\nhttps://a.test/4.svg\tany";
        let urls: Vec<String> = candidates(&Url::parse("https://a.test/").unwrap(), many).into_iter().map(String::from).collect();
        assert_eq!(urls, ["https://a.test/1.png", "https://a.test/4.svg", "https://a.test/2.png", "https://a.test/favicon.ico"]);
    }

    #[test]
    fn other_sites_count_only_when_the_page_loaded_from_them() {
        let page = Url::parse("https://a.test/").unwrap();
        let links = "https://cdn.b.test/icon.png\t192x192\nhttps://tracker.test/p.png\t512x512";
        let urls = |links: &str| candidates(&page, links).into_iter().map(String::from).collect::<Vec<_>>();
        assert_eq!(urls(links), ["https://a.test/favicon.ico"]);
        assert_eq!(urls(&format!("{links}\n\ncdn.b.test\nfonts.test")), ["https://cdn.b.test/icon.png", "https://a.test/favicon.ico"]);
        assert_eq!(urls("https://cdn.b.test:8443/i.png\t96x96\n\ncdn.b.test"), ["https://a.test/favicon.ico"]);
        assert_eq!(urls("https://evil.cdn.b.test/i.png\t96x96\n\ncdn.b.test"), ["https://a.test/favicon.ico"]);
        let crowd: String = (0..MAX_LOADED_HOSTS).map(|i| format!("\nh{i}.test")).collect();
        assert_eq!(urls(&format!("https://cdn.b.test/i.png\t96x96\n{crowd}\ncdn.b.test")), ["https://a.test/favicon.ico"]);
    }

    #[test]
    fn subdomains_fall_back_to_their_site_only() {
        assert_eq!(lookup_hosts("en.m.wikipedia.org"), ["en.m.wikipedia.org", "m.wikipedia.org", "wikipedia.org", "www.wikipedia.org"]);
        assert_eq!(lookup_hosts("www.a.test"), ["www.a.test", "a.test"]);
        assert_eq!(lookup_hosts("a.test"), ["a.test", "www.a.test"]);
        assert_eq!(lookup_hosts("bob.github.io"), ["bob.github.io", "www.bob.github.io"]);
        assert_eq!(lookup_hosts("localhost"), ["localhost"]);
        assert_eq!(lookup_hosts("www.a.test."), ["www.a.test", "a.test"]);
        assert_eq!(lookup_hosts("10.0.0.1"), ["10.0.0.1"]);
    }

    #[test]
    fn normalize_squares_and_rejects_bad_input() {
        for (w, h) in [(16, 16), (180, 90)] {
            let out = image::load_from_memory(&normalize(&encoded(w, h)).unwrap()).unwrap();
            assert_eq!((out.width(), out.height()), (SIZE, SIZE));
        }
        assert!(normalize(&STANDARD.encode("<svg xmlns='http://www.w3.org/2000/svg'/>")).is_none());
        assert!(normalize(&STANDARD.encode(vec![0u8; MAX_BYTES + 1])).is_none());
        assert!(normalize("not base64!").is_none());
        assert!(normalize(&encoded(MAX_DIMENSION + 1, 1)).is_none());
    }

    #[test]
    fn dark_marks_on_transparency_get_a_light_plate() {
        let mut mark = RgbaImage::new(32, 32);
        for y in 8..24 {
            for x in 8..24 {
                mark.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        assert!(is_dark_on_clear(&mark));
        assert!(!is_dark_on_clear(&RgbaImage::from_pixel(32, 32, Rgba([0, 0, 0, 255]))));
        assert!(!is_dark_on_clear(&RgbaImage::from_pixel(32, 32, Rgba([250, 250, 250, 255]))));
        let mut png = Vec::new();
        mark.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();
        let out = image::load_from_memory(&normalize(&STANDARD.encode(png)).unwrap()).unwrap().to_rgba8();
        assert_eq!(out.get_pixel(0, 0)[3], 255);
    }
}

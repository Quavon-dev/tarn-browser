#![forbid(unsafe_code)]

mod prepare;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs, process};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ed25519_dalek::{Signer, SigningKey};
use shield::bundle::{FORMAT, ListEntry, MANIFEST, MAX_BUNDLE_BYTES, Manifest, SIGNATURE, sha256_hex, valid_name};

use crate::prepare::{ENTRIES, Prepared};

const USAGE: &str = "usage: filter-bundle sources <sources.json>\n       \
    filter-bundle prepare --sources <sources.json> --raw <dir> --youtube <youtube.json> [--theme-revocations <json>] [--previous <dir>] [--allow-change] --out <dir>\n       \
    filter-bundle build --in <dir> --version <n> --out <dir>\n       \
    filter-bundle keygen\n       \
    filter-bundle catalog [--kind extension|theme] --in <index.json> --key <seed file> --out <dir>";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("keygen") if args.len() == 1 => keygen(),
        Some("sources") if args.len() == 2 => prepare::load_sources(Path::new(&args[1])).map(|s| {
            for source in s.sources {
                println!("{}\t{}", source.name, source.url);
            }
        }),
        Some("prepare") => {
            flags(&args[1..], &["--sources", "--raw", "--youtube", "--theme-revocations", "--previous", "--out"], &["--allow-change"]).and_then(|f| {
                let path = |k: &str| f.get(k).map(PathBuf::from).ok_or_else(|| USAGE.to_owned());
                prepare::prepare(&prepare::Options {
                    sources: &path("--sources")?,
                    raw: &path("--raw")?,
                    youtube: &path("--youtube")?,
                    theme_revocations: path("--theme-revocations").ok().as_deref(),
                    previous: path("--previous").ok().as_deref(),
                    allow_change: f.contains_key("--allow-change"),
                    out: &path("--out")?,
                })
                .map(drop)
            })
        }
        Some("build") => parse_build(&args[1..]).and_then(|(input, version, out)| {
            let seed = env::var("FILTER_SIGNING_KEY").map_err(|_| "FILTER_SIGNING_KEY is not set".to_owned())?;
            build(&input, version, &out, &signing_key(&seed)?, &now_rfc3339())
        }),
        Some("catalog") => flags(&args[1..], &["--kind", "--in", "--key", "--out"], &[]).and_then(|f| {
            let catalog = match f.get("--kind").map(String::as_str) {
                None | Some("extension") => &shield::catalog::EXTENSIONS,
                Some("theme") => &shield::catalog::THEMES,
                Some(_) => return Err(USAGE.to_owned()),
            };
            match (f.get("--in"), f.get("--key"), f.get("--out")) {
                (Some(input), Some(key), Some(out)) => {
                    let seed = fs::read_to_string(key).map_err(|e| format!("{key}: {e}"))?;
                    sign_catalog(catalog, Path::new(input), &signing_key(&seed)?, Path::new(out))
                }
                _ => Err(USAGE.to_owned()),
            }
        }),
        _ => Err(USAGE.to_owned()),
    };
    if let Err(e) = result {
        eprintln!("filter-bundle: {e}");
        process::exit(1);
    }
}

/// Signs a catalog index with the catalog's own key, after checking that the app would accept it.
fn sign_catalog(catalog: &shield::catalog::Catalog, input: &Path, key: &SigningKey, out: &Path) -> Result<(), String> {
    let index = fs::read(input).map_err(|e| format!("{}: {e}", input.display()))?;
    let signature = key.sign(&catalog.signed_message(&index)).to_bytes();
    shield::catalog::verify_catalog(catalog, &index, &signature, &key.verifying_key().to_bytes()).map_err(|e| e.to_string())?;
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    fs::write(out.join("index.json"), &index).map_err(|e| e.to_string())?;
    fs::write(out.join("index.sig"), signature).map_err(|e| e.to_string())
}

fn keygen() -> Result<(), String> {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| e.to_string())?;
    let key = SigningKey::from_bytes(&seed);
    println!("FILTER_SIGNING_KEY={}", BASE64.encode(seed));
    println!("public key (base64): {}", BASE64.encode(key.verifying_key().to_bytes()));
    println!("public key (bytes): {:?}", key.verifying_key().to_bytes());
    Ok(())
}

fn signing_key(seed_b64: &str) -> Result<SigningKey, String> {
    let seed: [u8; 32] = BASE64.decode(seed_b64.trim()).ok().and_then(|s| s.try_into().ok()).ok_or("FILTER_SIGNING_KEY must be a base64 32-byte seed")?;
    Ok(SigningKey::from_bytes(&seed))
}

/// `--flag value` pairs and bare `switches`, each at most once.
fn flags(args: &[String], valued: &[&str], switches: &[&str]) -> Result<std::collections::HashMap<String, String>, String> {
    let mut map = std::collections::HashMap::new();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let value = if switches.contains(&flag.as_str()) {
            String::new()
        } else if valued.contains(&flag.as_str()) {
            it.next().ok_or(USAGE)?.clone()
        } else {
            return Err(USAGE.to_owned());
        };
        if map.insert(flag.clone(), value).is_some() {
            return Err(USAGE.to_owned());
        }
    }
    Ok(map)
}

fn parse_build(args: &[String]) -> Result<(PathBuf, u64, PathBuf), String> {
    let f = flags(args, &["--in", "--version", "--out"], &[])?;
    let version = f.get("--version").and_then(|v| v.parse().ok());
    match (f.get("--in"), version, f.get("--out")) {
        (Some(input), Some(version), Some(out)) => Ok((input.into(), version, out.into())),
        _ => Err(USAGE.to_owned()),
    }
}

/// Hashes and signs the files `prepare` wrote; parses nothing but its small `entries.json`.
fn build(input: &Path, version: u64, out: &Path, key: &SigningKey, created: &str) -> Result<(), String> {
    let index = shield::cache::read_capped(fs::File::open(input.join(ENTRIES)).map_err(|e| format!("{ENTRIES}: {e}"))?, 64 << 10)
        .map_err(|e| format!("{ENTRIES}: {e}"))?;
    let prepared: Vec<Prepared> = serde_json::from_slice(&index).map_err(|e| format!("{ENTRIES}: {e}"))?;
    fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut entries: Vec<ListEntry> = Vec::new();
    for p in prepared {
        if !valid_name(&p.name) || entries.iter().any(|e| e.name == p.name) {
            return Err(format!("{}: list names must be unique [a-z0-9._-]+ file names", p.name));
        }
        let data = fs::read(input.join(&p.name)).map_err(|e| format!("{}: {e}", p.name))?;
        if data.len() as u64 > p.kind.max_bytes() {
            return Err(format!("{}: {} bytes exceed the limit for its kind", p.name, data.len()));
        }
        fs::write(out.join(&p.name), &data).map_err(|e| format!("{}: {e}", p.name))?;
        entries.push(ListEntry { name: p.name, sha256: sha256_hex(&data), size: data.len() as u64, category: p.category, kind: p.kind, count: p.count });
    }
    if entries.iter().map(|e| e.size).sum::<u64>() > MAX_BUNDLE_BYTES {
        return Err("the bundle exceeds its size limit".into());
    }
    let manifest = Manifest { format: FORMAT, version, created: created.to_owned(), lists: entries };
    let manifest = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(out.join(MANIFEST), &manifest).map_err(|e| format!("{MANIFEST}: {e}"))?;
    fs::write(out.join(SIGNATURE), key.sign(&manifest).to_bytes()).map_err(|e| format!("{SIGNATURE}: {e}"))
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    rfc3339(secs)
}

/// Howard Hinnant's days-to-civil algorithm, to avoid a date crate for one timestamp.
fn rfc3339(secs: u64) -> String {
    let (days, rem) = ((secs / 86_400) as i64 + 719_468, secs % 86_400);
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shield::bundle::{Kind, verify};

    #[test]
    fn signed_catalog_verifies_and_a_malformed_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let key = SigningKey::from_bytes(&[3; 32]);
        let input = dir.path().join("in.json");
        fs::write(&input, r#"{"kind":"tarn-extension-catalog","version":1,"created":1,"extensions":[]}"#).unwrap();
        let extensions = &shield::catalog::EXTENSIONS;
        sign_catalog(extensions, &input, &key, &dir.path().join("out")).unwrap();
        let index = fs::read(dir.path().join("out/index.json")).unwrap();
        let sig = fs::read(dir.path().join("out/index.sig")).unwrap();
        assert!(shield::catalog::verify(&index, &sig, &key.verifying_key().to_bytes()).is_ok());
        assert!(sign_catalog(&shield::catalog::THEMES, &input, &key, &dir.path().join("wrong")).is_err(), "an extension index is not a theme index");
        fs::write(&input, r#"{"kind":"tarn-theme-catalog","version":1,"created":1,"themes":[]}"#).unwrap();
        sign_catalog(&shield::catalog::THEMES, &input, &key, &dir.path().join("themes")).unwrap();
        let (index, sig) = (fs::read(dir.path().join("themes/index.json")).unwrap(), fs::read(dir.path().join("themes/index.sig")).unwrap());
        assert!(shield::catalog::verify_catalog(&shield::catalog::THEMES, &index, &sig, &key.verifying_key().to_bytes()).is_ok());
        fs::write(&input, r#"{"kind":"other","version":1,"created":1,"extensions":[]}"#).unwrap();
        assert!(sign_catalog(extensions, &input, &key, &dir.path().join("bad")).is_err());
    }

    #[test]
    fn formats_rfc3339() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_794_441_599), "2026-11-11T23:59:59Z");
    }

    #[test]
    fn parses_build_args() {
        let args: Vec<String> = ["--in", "prepared", "--version", "7", "--out", "dir"].map(String::from).into();
        let (input, version, out) = parse_build(&args).unwrap();
        assert_eq!((input, version, out), (PathBuf::from("prepared"), 7, PathBuf::from("dir")));
        assert!(parse_build(&["--in".into(), "a".into()]).is_err());
        assert!(parse_build(&["--version".into(), "x".into()]).is_err());
        let twice: Vec<String> = ["--in", "a", "--in", "b", "--version", "1", "--out", "o"].map(String::from).into();
        assert!(parse_build(&twice).is_err());
    }

    #[test]
    fn rejects_bad_seed() {
        assert!(signing_key("not base64!").is_err());
        assert!(signing_key(&BASE64.encode([1u8; 31])).is_err());
        assert!(signing_key(&BASE64.encode([1u8; 32])).is_ok());
    }

    fn prepared(dir: &Path, entries: &[(&str, Kind, &[u8])]) {
        fs::create_dir_all(dir).unwrap();
        let index: Vec<Prepared> = entries
            .iter()
            .map(|(name, kind, data)| {
                fs::write(dir.join(name), data).unwrap();
                Prepared { name: (*name).into(), category: "ads".into(), kind: *kind, count: 1 }
            })
            .collect();
        fs::write(dir.join(ENTRIES), serde_json::to_vec(&index).unwrap()).unwrap();
    }

    #[test]
    fn a_bundle_with_theme_withdrawals_verifies_and_carries_them() {
        use shield::revocations;
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in");
        fs::create_dir_all(&input).unwrap();
        let list = format!(r#"{{"sha256":["{}"]}}"#, "e".repeat(64));
        fs::write(input.join(revocations::FILE), &list).unwrap();
        let index = [Prepared { name: revocations::FILE.into(), category: revocations::CATEGORY.into(), kind: Kind::Config, count: 1 }];
        fs::write(input.join(ENTRIES), serde_json::to_vec(&index).unwrap()).unwrap();
        let out = dir.path().join("out");
        let key = SigningKey::from_bytes(&[3; 32]);
        build(&input, 7, &out, &key, "2026-11-15T00:00:00Z").unwrap();
        let read = |name: &str, _: u64| fs::read(out.join(name)).ok();
        let bundle = verify(&read(MANIFEST, 0).unwrap(), &read(SIGNATURE, 0).unwrap(), &read, &key.verifying_key().to_bytes(), None).unwrap();
        let carried = bundle.of(revocations::CATEGORY, Kind::Config).next().and_then(|l| revocations::parse(&l.data)).unwrap();
        assert_eq!(carried.sha256, ["e".repeat(64)]);
    }

    #[test]
    fn built_bundle_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in");
        prepared(&input, &[("easylist.txt", Kind::Filters, b"||tracker.example^\n"), ("ads.set", Kind::Set, &[0; 12])]);
        let out = dir.path().join("out");
        let key = SigningKey::from_bytes(&[3; 32]);
        build(&input, 42, &out, &key, "2026-11-12T00:00:00Z").unwrap();

        let read = |name: &str, _: u64| fs::read(out.join(name)).ok();
        let manifest = read(MANIFEST, 0).unwrap();
        let sig = read(SIGNATURE, 0).unwrap();
        let bundle = verify(&manifest, &sig, &read, &key.verifying_key().to_bytes(), Some(41)).unwrap();
        assert_eq!(bundle.version, 42);
        assert_eq!((bundle.lists[0].name.as_str(), bundle.lists[1].kind), ("easylist.txt", Kind::Set));

        let bad = dir.path().join("bad");
        prepared(&bad, &[("Bad.txt", Kind::Filters, b"x")]);
        assert!(build(&bad, 1, &out, &key, "").is_err());
        let twice = dir.path().join("twice");
        prepared(&twice, &[("a.txt", Kind::Filters, b"x"), ("a.txt", Kind::Filters, b"x")]);
        assert!(build(&twice, 1, &out, &key, "").is_err());
        let big = dir.path().join("big");
        prepared(&big, &[("youtube.json", Kind::Config, &vec![b' '; (Kind::Config.max_bytes() + 1) as usize])]);
        assert!(build(&big, 1, &out, &key, "").is_err());
    }
}

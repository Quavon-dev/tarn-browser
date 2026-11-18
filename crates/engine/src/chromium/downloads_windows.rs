//! Downloads of Chromium tabs on Windows: saved to the user's Downloads folder under a cleaned,
//! unused name, marked as coming from the internet, never opened. Chromium pauses and resumes
//! them itself. One from a host on the malware list gets no path until the user keeps it.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use cef::*;
use url::Url;

use crate::downloads::{flagged_source, folder_for, free_path, malware_warning, sanitize_filename, sorted_dir};
use crate::page::serialized_origin;
use crate::prompts::Answer;
use crate::{Event, Prompt, PromptKind};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

struct Download {
    id: u64,
    sink: Rc<dyn Fn(Event)>,
    path: PathBuf,
    url: Option<Url>,
    started: SystemTime,
    /// Chromium's handle for pausing, resuming and cancelling; known from its first report.
    control: Option<DownloadItemCallback>,
    done: u64,
    total: Option<u64>,
    paused: bool,
    reported: Instant,
}

impl Download {
    fn progress(&self) -> Event {
        Event::DownloadProgress { id: self.id, path: self.path.clone(), done: self.done, total: self.total, paused: self.paused }
    }
}

/// A download waiting at the malware prompt; dropping `callback` unanswered cancels it.
struct Held {
    sink: Rc<dyn Fn(Event)>,
    url: Option<Url>,
    name: String,
    callback: BeforeDownloadCallback,
    control: Option<DownloadItemCallback>,
}

/// The tab a download starts in, which asks about one from a host on the malware list.
pub(crate) struct Asker {
    pub sink: Rc<dyn Fn(Event)>,
    pub silenced: Rc<Cell<bool>>,
    pub profile: String,
    pub page: Option<Url>,
}

/// Chromium numbers downloads per profile, so a download is its store and its id.
type Key = (String, u32);

thread_local! {
    static DOWNLOADS: RefCell<HashMap<Key, Download>> = RefCell::new(HashMap::new());
    static HELD: RefCell<HashMap<Key, Held>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    static SORTED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn active(store: &str) -> usize {
    DOWNLOADS.with_borrow(|d| d.keys().filter(|(s, _)| s == store).count())
}

fn report(sink: &Rc<dyn Fn(Event)>, store: &str) {
    sink(Event::DownloadsActive(active(store)));
}

/// New downloads of `store` go into a Downloads subfolder by type.
pub(crate) fn set_sort(store: &str, on: bool) {
    SORTED.with_borrow_mut(|s| if on { s.insert(store.to_owned()) } else { s.remove(store) });
}

/// Chooses where a new download goes and starts it there without a dialog, sorted by type when the
/// store asks for it. One whose URL, first URL or page is on the malware list first waits for the
/// user to keep it. Returning without continuing `callback` cancels it.
pub(crate) fn begin(sink: Rc<dyn Fn(Event)>, store: &str, item: &DownloadItem, suggested: &str, callback: &BeforeDownloadCallback, tab: Asker) {
    let name = sanitize_filename(suggested);
    let parse = |s: CefStringUserfree| Url::parse(&CefString::from(&s).to_string()).ok();
    let url = parse(item.url());
    let Some(shield) = super::filter::shield_for(&tab.profile) else { return };
    let flagged = flagged_source([parse(item.original_url()), url.clone(), tab.page], &*shield.site_for);
    let url = url.map(|mut u| {
        let _ = u.set_username("");
        let _ = u.set_password(None);
        u
    });
    let key = (store.to_owned(), item.id());
    let Some(from) = flagged else { return start(sink, key, url, &name, callback, None) };
    let Some(origin) = serialized_origin(&from).filter(|_| !tab.silenced.get()) else { return };
    HELD.with_borrow_mut(|h| h.insert(key.clone(), Held { sink, url, name: name.clone(), callback: callback.clone(), control: None }));
    let reply: Answer = Box::new(move |answer: Option<_>| decide(key, answer.is_some()));
    let kind = PromptKind::MaliciousDownload { file: name };
    (tab.sink)(Event::Prompt(Prompt::answered(origin, None, false, malware_warning(&from), kind, reply, tab.silenced)));
}

/// The malware prompt's answer: a kept download starts, any other is cancelled without a notice.
fn decide(key: Key, keep: bool) {
    let Some(held) = HELD.with_borrow_mut(|h| h.remove(&key)) else { return };
    if keep {
        let control = held.control.clone();
        start(held.sink, key, held.url, &held.name, &held.callback, held.control);
        if let Some(control) = control {
            control.resume();
        }
    } else if let Some(control) = held.control {
        control.cancel();
    }
}

fn start(sink: Rc<dyn Fn(Event)>, key: Key, url: Option<Url>, name: &str, callback: &BeforeDownloadCallback, control: Option<DownloadItemCallback>) {
    let Some(downloads) = std::env::home_dir().map(|home| home.join("Downloads")) else {
        sink(Event::DownloadFailed { id: None, message: "Tarn could not find the Downloads folder.".into() });
        return;
    };
    let sort = SORTED.with_borrow(|s| s.contains(&key.0));
    let dir = sort.then(|| folder_for(name)).flatten().and_then(|f| sorted_dir(&downloads, f)).unwrap_or(downloads);
    let reserved: HashSet<PathBuf> = DOWNLOADS.with_borrow(|d| d.values().map(|d| d.path.clone()).collect());
    let path = free_path(&dir, name, |p| reserved.contains(p));
    let id = NEXT_ID.get();
    NEXT_ID.set(id + 1);
    let download = Download {
        id,
        sink: sink.clone(),
        path: path.clone(),
        url,
        started: SystemTime::now(),
        control,
        done: 0,
        total: None,
        paused: false,
        reported: Instant::now(),
    };
    let event = download.progress();
    let store = key.0.clone();
    DOWNLOADS.with_borrow_mut(|d| d.insert(key, download));
    callback.cont(Some(&CefString::from(path.to_string_lossy().as_ref())), 0);
    sink(event);
    report(&sink, &store);
}

/// Follows a download Tarn started: its progress at most twice a second and whenever it pauses
/// or resumes, then its end once. CEF also reports downloads before `on_before_download`, so one
/// Tarn has not started yet is left alone. An interrupted download is cancelled, so Chromium
/// cannot resume it after Tarn stopped tracking it and leave it unmarked. One held at the malware
/// prompt only keeps its handle, and is forgotten when it ends before the answer.
pub(crate) fn update(store: &str, item: &DownloadItem, callback: Option<&mut DownloadItemCallback>) {
    let key = (store.to_owned(), item.id());
    let interrupted = item.is_interrupted() == 1;
    let held = HELD.with_borrow_mut(|h| {
        let held = h.get_mut(&key)?;
        let fresh = callback.as_deref().filter(|_| held.control.is_none()).cloned();
        held.control = held.control.take().or_else(|| fresh.clone());
        Some(fresh)
    });
    if let Some(fresh) = held {
        if let Some(control) = fresh {
            control.pause();
        }
        if interrupted || item.is_canceled() == 1 {
            let gone = HELD.with_borrow_mut(|h| h.remove(&key));
            if interrupted {
                if let Some(callback) = callback {
                    callback.cancel();
                }
                if let Some(held) = gone {
                    (held.sink)(Event::DownloadFailed { id: None, message: format!("{} did not finish.", held.name) });
                }
            }
        }
        return;
    }
    let progress = DOWNLOADS.with_borrow_mut(|d| {
        let download = d.get_mut(&key)?;
        if let Some(callback) = &callback {
            download.control = Some((**callback).clone());
        }
        let paused = item.is_paused() == 1;
        download.done = u64::try_from(item.received_bytes()).unwrap_or(0);
        download.total = u64::try_from(item.total_bytes()).ok().filter(|t| *t > 0);
        let due = paused != download.paused || download.reported.elapsed() >= PROGRESS_INTERVAL;
        download.paused = paused;
        due.then(|| {
            download.reported = Instant::now();
            (download.sink.clone(), download.progress())
        })
    });
    if interrupted && let Some(callback) = callback {
        callback.cancel();
    }
    let (complete, failed) = (item.is_complete() == 1, item.is_canceled() == 1 || interrupted);
    if !complete && !failed {
        if let Some((sink, event)) = progress {
            sink(event);
        }
        return;
    }
    let Some(download) = DOWNLOADS.with_borrow_mut(|d| d.remove(&key)) else { return };
    let Download { id, sink, url, started, .. } = download;
    if complete {
        let path = Some(PathBuf::from(CefString::from(&item.full_path()).to_string())).filter(|p| !p.as_os_str().is_empty()).unwrap_or(download.path);
        let quarantined = mark_of_the_web(&path, url.as_ref().filter(|_| store.starts_with("p:")));
        sink(Event::DownloadFinished { id, url, path, started, quarantined });
    } else if item.is_canceled() == 1 && !interrupted {
        sink(Event::DownloadCancelled(id));
    } else {
        sink(Event::DownloadFailed { id: Some(id), message: "The download did not finish.".into() });
    }
    report(&sink, store);
}

fn control(store: &str, id: u64, act: impl FnOnce(&DownloadItemCallback)) {
    let found = DOWNLOADS.with_borrow(|d| d.iter().find(|((s, _), dl)| s == store && dl.id == id).and_then(|(_, dl)| dl.control.clone()));
    if let Some(control) = found {
        act(&control);
    }
}

/// `id` is from `Event::DownloadProgress`.
pub(crate) fn pause(store: &str, id: u64) {
    control(store, id, |c| c.pause());
}

pub(crate) fn resume(store: &str, id: u64) {
    control(store, id, |c| c.resume());
}

pub(crate) fn cancel(store: &str, id: u64) {
    control(store, id, |c| c.cancel());
}

/// For quit: Chromium cannot continue a download after the next launch, so every one of `store`,
/// held at the malware prompt or not, is cancelled and Chromium removes its part.
pub(crate) fn quit(store: &str) {
    let held: Vec<Held> = HELD.with_borrow_mut(|h| h.extract_if(|(s, _), _| s == store).map(|(_, held)| held).collect());
    let mut controls: Vec<DownloadItemCallback> = held.into_iter().filter_map(|h| h.control).collect();
    controls.extend(DOWNLOADS.with_borrow(|d| d.iter().filter(|((s, _), _)| s == store).filter_map(|(_, dl)| dl.control.clone()).collect::<Vec<_>>()));
    for control in controls {
        control.cancel();
    }
}

/// Writes the `Zone.Identifier` stream that makes Windows and SmartScreen treat the file as
/// downloaded from the internet. Only a web source is named; private stores and other sources
/// say `about:internet`, as Chromium does.
fn mark_of_the_web(path: &Path, from: Option<&Url>) -> bool {
    if !path.is_file() {
        return false;
    }
    let source = from.filter(|u| matches!(u.scheme(), "http" | "https")).map_or_else(|| "about:internet".to_owned(), Url::to_string);
    let stream = PathBuf::from(format!("{}:Zone.Identifier", path.display()));
    std::fs::write(stream, format!("[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={source}\r\n")).is_ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn downloads_are_marked_with_web_sources_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.zip");
        std::fs::write(&file, b"x").unwrap();
        let stream = || std::fs::read_to_string(format!("{}:Zone.Identifier", file.display())).unwrap();
        let from = url::Url::parse("https://example.com/a.zip").unwrap();
        assert!(super::mark_of_the_web(&file, Some(&from)));
        assert!(stream().contains("ZoneId=3") && stream().contains("HostUrl=https://example.com/a.zip"), "{}", stream());
        let data = url::Url::parse("data:text/plain,secret").unwrap();
        assert!(super::mark_of_the_web(&file, Some(&data)) && stream().contains("HostUrl=about:internet"));
        assert!(!super::mark_of_the_web(&dir.path().join("missing.zip"), None));
    }
}

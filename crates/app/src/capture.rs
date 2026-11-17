use std::path::PathBuf;
use std::rc::Rc;

use browser_core::capture::file_name;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{ClipboardItem, Context, Div, Image, ImageFormat, InteractiveElement, actions};
use url::Url;

use crate::browser::{Browser, DownloadNotice, TabKey, now};

actions!(tarn, [TakeScreenshot, SavePdf, CopyMarkdownLink, CopyTitle, PictureInPicture]);

pub fn listen(root: Div, cx: &mut Context<Browser>) -> Div {
    root.on_action(cx.listener(|b, _: &TakeScreenshot, _, cx| b.capture(true, cx)))
        .on_action(cx.listener(|b, _: &SavePdf, _, cx| b.capture(false, cx)))
        .on_action(cx.listener(|b, _: &CopyMarkdownLink, _, cx| b.copy_page(true, cx)))
        .on_action(cx.listener(|b, _: &CopyTitle, _, cx| b.copy_page(false, cx)))
        .on_action(cx.listener(|b, _: &PictureInPicture, _, cx| b.picture_in_picture(cx)))
}

pub fn markdown_link(title: &str, url: &str) -> String {
    let title = ui::text::bounded_title(title);
    let text = if title.is_empty() { url } else { &title };
    let mut escaped = String::new();
    for c in text.chars() {
        if matches!(c, '\\' | '[' | ']') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    format!("[{escaped}]({})", url.replace('(', "%28").replace(')', "%29").replace(' ', "%20"))
}

impl Browser {
    fn page_title(&self, key: TabKey) -> String {
        self.views.get(&key).map(|v| v.title()).filter(|t| !t.is_empty()).or_else(|| self.model.tab(key).map(|t| t.title.clone())).unwrap_or_default()
    }

    /// Screenshot of the visible part of the active tab, or the whole page as PDF, saved to
    /// Downloads and listed in the tab's own profile store (in memory for private spaces).
    pub fn capture(&mut self, screenshot: bool, cx: &mut Context<Self>) {
        let (Some(key), Some(view)) = (self.model.active, self.active_view()) else { return };
        let Some(profile_id) = self.profile_of(key) else { return };
        let page = (profile_id, self.tab_url(key).unwrap_or_default(), self.page_title(key));
        let (tx, mut rx) = mpsc::unbounded();
        if screenshot {
            view.snapshot_png(None, move |png| drop(tx.unbounded_send(png)));
        } else {
            view.pdf(move |pdf| drop(tx.unbounded_send(pdf)));
        }
        let started = now();
        cx.spawn(async move |this, cx| {
            if let Some(bytes) = rx.next().await {
                this.update(cx, |b, cx| b.save_capture(key.private, page, screenshot, started, bytes, cx)).ok();
            }
        })
        .detach();
    }

    /// `page` is the tab's profile, URL and title when the capture started; the tab may be gone, or
    /// its id taken by a new tab, once the bytes arrive.
    fn save_capture(
        &mut self,
        private: bool,
        page: (store::Id, String, String),
        screenshot: bool,
        started: i64,
        bytes: Option<Vec<u8>>,
        cx: &mut Context<Self>,
    ) {
        let (profile_id, url, title) = page;
        let name = file_name(&title, Url::parse(&url).ok().as_ref(), if screenshot { "png" } else { "pdf" });
        let result = bytes
            .ok_or_else(|| if cfg!(windows) { "Chromium could not capture the page" } else { "WebKit could not capture the page" }.to_owned())
            .and_then(|bytes| {
                if screenshot {
                    cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, bytes.clone())));
                }
                write_download(&name, &bytes).map_err(|e| e.to_string())
            });
        if let Ok(path) = &result
            && self.core.keeps_record_since(private, profile_id, started)
        {
            self.store(private).add_download(profile_id, &url, &path.to_string_lossy(), "finished", now()).ok();
            self.downloads_popover.changed += 1;
        }
        match result {
            Ok(path) if screenshot => self.show_toast_as(browser_core::toast::Tone::Success, "Screenshot saved", Some(crate::toast::Action::Reveal(path)), cx),
            result => {
                self.download = Some(DownloadNotice { private, profile_id, result: result.map(|path| (path, true)) });
                cx.notify();
            }
        }
    }

    fn copy_page(&mut self, markdown: bool, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        let title = self.page_title(key);
        let text = if markdown { markdown_link(&title, &self.tab_url(key).unwrap_or_default()) } else { ui::text::bounded_title(&title) };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_toast_as(browser_core::toast::Tone::Success, if markdown { "Link copied" } else { "Title copied" }, None, cx);
    }

    fn picture_in_picture(&mut self, cx: &mut Context<Self>) {
        let (Some(key), Some(view)) = (self.model.active, self.active_view()) else { return };
        let (weak, (tx, mut rx)) = (Rc::downgrade(&view), mpsc::unbounded());
        view.picture_in_picture(move |result| drop(tx.unbounded_send(result)));
        cx.spawn(async move |this, cx| {
            let notice = match rx.next().await.as_deref() {
                Some("none") => "This page has no video",
                Some("refused") => "This video cannot play in Picture in Picture",
                _ => return,
            };
            this.update(cx, |b, cx| {
                if b.same_view(key, &weak) {
                    b.notify_offer(key, notice.into(), None);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

pub(crate) fn write_download(name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = std::env::home_dir().ok_or(std::io::ErrorKind::NotFound)?.join("Downloads");
    browser_core::capture::save_new(&dir, name, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_links_escape_brackets_and_parentheses() {
        assert_eq!(markdown_link("Rust [book]", "https://a.test/x"), r"[Rust \[book\]](https://a.test/x)");
        assert_eq!(markdown_link(r"a\]b", "https://a.test/f(1)"), r"[a\\\]b](https://a.test/f%281%29)");
        assert_eq!(markdown_link("", "https://a.test/"), "[https://a.test/](https://a.test/)");
        assert_eq!(markdown_link("Line\nbreak", "https://a.test/"), "[Line break](https://a.test/)");
    }
}

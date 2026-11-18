//! Page translation on this Mac (roadmap item 10): offered in the notice banner when a page
//! declares a language other than the user's, never under Lockdown.

use std::rc::Rc;

use engine::translation::{self, Step, TranslateError};
use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{AsyncApp, Context, WeakEntity};
use shield::ShieldLevel;
use url::Url;

use crate::blocking::Offer;
use engine::WebView;

use crate::browser::{Browser, TabKey};

impl Browser {
    /// After a page finished: offers translation once per host, profile and session, unless the
    /// tab shows another notice.
    pub(crate) fn offer_translation(&mut self, key: TabKey, url: &Url, cx: &mut Context<Self>) {
        let (Some(host), Some(profile)) = (url.host_str(), self.profile_of(key)) else { return };
        let offered = (key.private, profile, host.to_owned());
        if !translation::supported()
            || self.level_for_tab(key, url.as_str()) == ShieldLevel::Lockdown
            || self.pages.get(&key).is_some_and(|p| p.notice.is_some())
            || self.translation_offered.contains(&offered)
        {
            return;
        }
        let Some(view) = self.views.get(&key) else { return };
        let preferred = translation::preferred_languages();
        let (tx, mut rx) = mpsc::unbounded();
        view.translation_step(&Step::Language, move |lang| drop(tx.unbounded_send(lang)));
        let page = url.clone();
        cx.spawn(async move |this, cx| {
            let Some(source) = rx.next().await.and_then(|lang| translation::offer(&lang, &preferred)) else { return };
            this.update(cx, |b, cx| {
                if !b.shows(key, None, &page) || b.pages.get(&key).is_some_and(|p| p.notice.is_some()) || !b.translation_offered.insert(offered) {
                    return;
                }
                let message = format!("This page is in {}. Translate it on this Mac?", translation::language_name(&source));
                b.notify_offer(key, message, Some(Offer::Translate(source, page)));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Whether the tab still shows `page`, in `view` when given.
    fn shows(&self, key: TabKey, view: Option<&Rc<WebView>>, page: &Url) -> bool {
        self.views.get(&key).is_some_and(|v| view.is_none_or(|view| Rc::ptr_eq(v, view)) && v.url().as_ref() == Some(page))
    }

    /// Reads the page's text nodes, translates them in batches and writes each batch back as
    /// text while the tab still shows the page the offer was made for.
    pub(crate) fn translate_page(&mut self, key: TabKey, source: String, page: Url, cx: &mut Context<Self>) {
        let preferred = translation::preferred_languages();
        let (Some(view), Some(target)) = (self.views.get(&key).filter(|_| self.shows(key, None, &page)).cloned(), preferred.first().cloned()) else { return };
        let language = translation::language_name(&source);
        self.notify_offer(key, format!("Translating from {language}…"), None);
        cx.notify();
        let (tx, mut rx) = mpsc::unbounded();
        view.translation_step(&Step::Read, move |json| drop(tx.unbounded_send(json)));
        cx.spawn(async move |this, cx| {
            let still_shown = |this: &WeakEntity<Browser>, cx: &mut AsyncApp| this.update(cx, |b, _| b.shows(key, Some(&view), &page)).unwrap_or(false);
            let texts: Vec<String> = rx.next().await.and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default();
            for (start, batch) in translation::batches(texts) {
                let (tx, rx) = oneshot::channel();
                translation::translate(&batch, &source, &target, move |result| drop(tx.send(result)));
                let Ok(result) = rx.await else { return };
                if !still_shown(&this, cx) {
                    return;
                }
                let notice = match result {
                    Ok(texts) => {
                        view.translation_step(&Step::Write { start, texts }, |_| {});
                        continue;
                    }
                    Err(TranslateError::NotInstalled) => (
                        format!(
                            "Translating from {language} needs its language pack. Add it in System Settings › General › Language & Region › Translation Languages."
                        ),
                        None,
                    ),
                    Err(TranslateError::Failed) if start > 0 => ("Part of this page could not be translated.".to_owned(), Some(Offer::ShowOriginal)),
                    Err(TranslateError::Failed) => ("This page could not be translated.".to_owned(), None),
                };
                this.update(cx, |b, cx| {
                    b.notify_offer(key, notice.0, notice.1);
                    cx.notify();
                })
                .ok();
                return;
            }
            if still_shown(&this, cx) {
                this.update(cx, |b, cx| {
                    b.notify_offer(key, format!("Translated from {language}"), Some(Offer::ShowOriginal));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }
}

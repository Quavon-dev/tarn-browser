use std::path::PathBuf;

use browser_core::extensions::install::{self, Fetched, Pending, Slot, StoreCatalog, StoreContext};
use browser_core::extensions::{EVERY_SITE_SETTING, Install, badge, every_site_setting, may_use_all_sites};
use browser_core::github::{Asset, Hop};
use browser_core::tabs::TabKey;
use browser_core::theme_fetch::{self as fetch, shown};
use nsh_base::input;
use nsh_core::picker::FilePickRequest;
use nsh_core::prelude::*;
use shield::catalog::Index;
use store::{Id, InstalledExtension, Store};

use crate::controls::{accent_pill, group, label, pill, secure_button, switch_row};
use crate::theme::{Secure, TEXT_SMALL, Theme};

/// What a client gives the Extensions pane.
pub trait Host: Copy + 'static {
    type Transport: Hop;
    fn transport() -> Self::Transport;
    fn with_store<R>(self, f: impl FnOnce(&Store) -> R) -> R;
    /// The persistent profile shown and its data-store seed; `None` in a private space or burner tab.
    fn profile(self) -> Option<(Id, String)>;
    /// Where installs unpack: `<root>/<profile id>/<install id>/<n>`.
    fn root(self) -> PathBuf;
    fn catalog_dir(self) -> PathBuf;
    /// A new install id: 32 lowercase hex digits.
    fn new_id(self) -> String;
    fn ui_locales(self) -> Vec<String>;
    /// Forgets the install's site answers, stored and of this session (`Runtime::forget_answers`).
    fn forget_answers(self, profile: Id, install_id: &str) -> Result<(), String>;
    /// The installs of the profile changed: the client reloads its extension runtime.
    fn changed(self);
    /// A line the client wants shown above the list, such as a missing engine feature.
    fn notice(self) -> Option<String>;
    fn open_link(self, url: &'static str);
    fn store_link_allowed(self, tab: TabKey) -> bool;
}

#[derive(Clone, Default)]
struct State {
    message: Option<(bool, String)>,
    busy: bool,
    pending: Option<Pending>,
    choice: Option<(String, Vec<Asset>)>,
    catalog: Option<Index>,
    armed: Option<String>,
    offer: Option<install::StoreOffer>,
    confirmed: Option<String>,
}

#[derive(Clone, Default)]
struct Repo(String);

#[derive(Clone, Copy)]
struct Pane<H: Host> {
    host: H,
    state: Signal<State>,
    repo: Signal<Repo>,
}

fn detached(future: impl Future<Output = ()> + 'static) {
    Owner::detached().run(|| drop(spawn(future)));
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl<H: Host> Pane<H> {
    fn get<T>(self, f: impl FnOnce(&State) -> T) -> T {
        self.state.with(f)
    }

    fn set(self, f: impl FnOnce(&mut State)) {
        self.state.update(f);
    }

    fn say(self, error: bool, message: impl Into<String>) {
        let message = message.into();
        self.set(|s| s.message = Some((error, message)));
    }

    fn report(self, result: Result<(), String>, done: &str) {
        match result {
            Ok(()) => self.say(false, done),
            Err(e) => self.say(true, e),
        }
        self.host.changed();
    }

    fn rows(self) -> Vec<InstalledExtension> {
        self.host.profile().map(|(id, _)| self.host.with_store(|s| s.extensions(id).unwrap_or_default())).unwrap_or_default()
    }

    fn install(self, what: Install, update_of: Option<String>) {
        self.run(what, update_of, None, None);
    }

    fn run(self, what: Install, update_of: Option<String>, temp: Option<PathBuf>, confirmed: Option<String>) {
        let drop_temp = move || {
            if let Some(path) = &temp {
                std::fs::remove_file(path).ok();
            }
        };
        if self.state.with_untracked(|s| s.busy || s.pending.is_some()) {
            return drop_temp();
        }
        let Some((profile, seed)) = self.host.profile() else { return drop_temp() };
        if what.needs_network() && !self.host.with_store(fetch::downloads_on) {
            return self.say(true, "Theme and extension downloads are off in Privacy settings.");
        }
        let host = self.host;
        let slot = match host.with_store(|s| install::slot(s, profile, &host.root(), update_of.as_deref(), || host.new_id())) {
            Ok(slot) => slot,
            Err(e) => {
                drop_temp();
                return self.say(true, e);
            }
        };
        let revoked = host.with_store(install::revoked);
        let locales = host.ui_locales();
        self.set(|s| {
            s.busy = true;
            s.confirmed = confirmed;
            s.choice = None;
            s.message = Some((false, if what.needs_network() { "Downloading\u{2026}" } else { "Unpacking\u{2026}" }.to_owned()));
        });
        detached(async move {
            let work = slot.clone();
            let result = blocking(move || install::fetch(&H::transport(), what, &work, &revoked, &seed, &locales)).await.unwrap_or_else(|e| Err(e.to_string()));
            drop_temp();
            self.fetched(slot, result);
        });
    }

    fn fetched(self, slot: Slot, result: Result<Fetched, String>) {
        let name = slot.previous.as_ref().map(|p| shown(&p.name));
        let confirmed = self.state.with_untracked(|s| s.confirmed.clone());
        self.set(|s| {
            s.busy = false;
            s.message = None;
            s.confirmed = None;
        });
        match result.and_then(|f| match f {
            Fetched::Offer(offer) => install::pending(slot, offer).map(Some),
            Fetched::Choose(input, assets) => {
                self.set(|s| s.choice = Some((input, assets)));
                Ok(None)
            }
            Fetched::Current => Err(format!("{}: this extension is up to date.", name.unwrap_or_default())),
        }) {
            Ok(Some(pending)) => {
                let agreed = confirmed.is_some_and(|id| install::store_confirmed(&id, &pending.row.source));
                self.set(|s| s.pending = Some(pending));
                if agreed {
                    self.confirm();
                }
            }
            Ok(None) => {}
            Err(e) => self.say(true, e),
        }
    }

    fn confirm(self) {
        let Some(pending) = self.state.with_untracked(|s| s.pending.clone()) else { return };
        self.set(|s| s.pending = None);
        let done = if pending.slot.previous.is_some() { "Updated." } else { "Installed. It asks on each site before it runs there." };
        self.report(self.host.with_store(|s| install::confirm(s, &pending)), done);
    }

    fn cancel(self) {
        if let Some(pending) = self.state.with_untracked(|s| s.pending.clone()) {
            pending.slot.discard();
        }
        self.set(|s| s.pending = None);
    }

    fn pick_file(self) {
        let host = self.host;
        let request = FilePickRequest::new(false, 1).filter(nsh_core::file_dialog::FileFilter::new("Extension", ["zip", "crx"]));
        detached(async move {
            let Some(file) = pick_files(request).await.into_iter().next() else { return };
            let path = std::env::temp_dir().join(format!("tarn-ext-{}.zip", host.new_id()));
            if let Err(e) = std::fs::write(&path, file.data.bytes()) {
                return self.say(true, format!("The file could not be read: {e}"));
            }
            self.run(Install::File(path.clone()), None, Some(path), None);
        });
    }

    fn show_official(self) {
        if self.state.with_untracked(|s| s.busy) {
            return;
        }
        if !self.host.with_store(fetch::downloads_on) {
            return self.say(true, "Theme and extension downloads are off in Privacy settings.");
        }
        let last = self.host.with_store(|s| s.setting(install::CATALOG_SEEN_SETTING).ok().flatten().and_then(|v| v.parse().ok()));
        let dir = self.host.catalog_dir();
        self.set(|s| {
            s.busy = true;
            s.message = Some((false, "Loading the official catalog\u{2026}".to_owned()));
        });
        detached(async move {
            let result = blocking(move || install::fetch_catalog(&H::transport(), &dir, last, now_secs(), &install::catalog_key()))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            self.set(|s| {
                s.busy = false;
                s.message = None;
            });
            match result {
                Ok(index) => {
                    self.host.with_store(|s| install::catalog_fetched(s, &index));
                    self.set(|s| s.catalog = Some(index));
                }
                Err(e) => self.say(true, e),
            }
        });
    }

    fn store_check<R>(self, tab: TabKey, busy: bool, step: impl FnOnce(StoreContext, StoreCatalog) -> R) -> R {
        let allowed = self.host.profile().is_some() && self.host.store_link_allowed(tab);
        let context = StoreContext { allowed, configured: install::catalog_configured(), downloads: self.host.with_store(fetch::downloads_on), busy };
        let (catalog, installed) = (self.state.with_untracked(|s| s.catalog.clone()), self.rows());
        let revoked = self.host.with_store(install::revoked);
        step(context, StoreCatalog { index: catalog.as_ref(), now: now_secs(), revoked: &revoked, installed: &installed })
    }

    fn confirm_offer(self) {
        let Some(install::StoreOffer { tab, entry: offered }) = self.state.with_untracked(|s| s.offer.clone()) else { return };
        self.set(|s| s.offer = None);
        let busy = self.state.with_untracked(|s| s.busy || s.pending.is_some() || s.choice.is_some());
        match self.store_check(tab, busy, |context, catalog| install::store_confirm(&offered, context, catalog)) {
            Ok(entry) => self.run(Install::Official(*entry), None, None, Some(offered.id)),
            Err(why) => self.say(true, why),
        }
    }

    fn install_official(self, id: &str) {
        let entry = self
            .state
            .with_untracked(|s| s.catalog.as_ref().filter(|c| !shield::catalog::is_old(c, now_secs()))?.extensions.iter().find(|e| e.id == id).cloned());
        match entry {
            Some(entry) => self.install(Install::Official(entry), None),
            None => self.say(true, "Show official extensions again first: the list is more than 30 days old."),
        }
    }

    fn update(self, row: &InstalledExtension) {
        let catalog = self.state.with_untracked(|s| s.catalog.clone());
        match install::update_of(row, catalog.as_ref()) {
            Some(what) => self.install(what, Some(row.install_id.clone())),
            None => self.say(false, format!("{}: this extension is up to date.", shown(&row.name))),
        }
    }

    fn remove(self, row: &InstalledExtension) {
        if self.state.with_untracked(|s| s.armed.as_deref() != Some(row.install_id.as_str())) {
            let id = row.install_id.clone();
            return self.set(|s| s.armed = Some(id));
        }
        self.set(|s| s.armed = None);
        let root = self.host.root();
        self.report(self.host.with_store(|s| install::remove(s, &root, row)), "Removed.");
    }
}

fn small(content: impl Into<SharedString>) -> Text {
    text(content.into()).text_size(px(TEXT_SMALL)).text_color(Theme::muted())
}

fn note(content: impl Into<SharedString>, error: bool) -> Text {
    small(content).text_color(if error { Secure::warning() } else { Theme::muted() })
}

fn section(name: &str) -> Text {
    label(name.to_owned()).font_weight(FontWeight::SEMIBOLD).pt_5().pb_2().px_1()
}

fn row() -> View {
    view().flex_row().flex_wrap().items_center().gap_2().px_3().py_2()
}

fn installed<H: Host>(pane: Pane<H>, r: InstalledExtension) -> View {
    let host = pane.host;
    let detail = format!("{} · {}", shown(&r.version), badge(&r.source));
    let (on, every) = (r.enabled, r.all_sites);
    let mut out = group().child(row().child(view().flex_col().flex_1().child(label(shown(&r.name)).truncate()).child(small(detail)))).child({
        let r = r.clone();
        switch_row("On in this profile", move || on, move || pane.report(host.with_store(|s| install::set_enabled(s, &r, !on)), ""))
    });
    if may_use_all_sites(&r.source) {
        let r = r.clone();
        out = out.child(switch_row("Every site it lists", move || every, move || pane.report(host.with_store(|s| install::set_all_sites(s, &r, !every)), "")));
    }
    let armed = pane.get(|s| s.armed.as_deref() == Some(r.install_id.as_str()));
    let (forget, update, remove) = (r.clone(), r.clone(), r.clone());
    let mut buttons = row().child(pill("Forget Site Answers", move || {
        let cleared = host.forget_answers(forget.profile_id, &forget.install_id);
        pane.report(cleared, "It asks again on every site.");
    }));
    if r.source != "file" {
        buttons = buttons.child(pill("Check for Update", move || pane.update(&update)));
    }
    buttons = buttons.child(pill(if armed { "Press Again to Remove" } else { "Remove" }, move || pane.remove(&remove)));
    out.child(buttons)
}

fn consent<H: Host>(pane: Pane<H>) -> Option<View> {
    let p = pane.get(|s| s.pending.clone())?;
    let action = if p.slot.previous.is_some() { "Update" } else { "Install" };
    let lines = p.lines().into_iter().map(|l| text(l).text_size(px(TEXT_SMALL)).text_color(Secure::muted()).into_element());
    Some(
        view().pt_3().child(
            crate::controls::secure_surface(Theme::radius_group())
                .p_3()
                .flex_col()
                .gap_2()
                .child(text(p.title()).font_weight(FontWeight::SEMIBOLD))
                .children(lines)
                .child(view().flex_row().gap_2().child(secure_button(action, move || pane.confirm(), true)).child(secure_button(
                    "Cancel",
                    move || pane.cancel(),
                    false,
                ))),
        ),
    )
}

fn store_offer<H: Host>(pane: Pane<H>) -> Option<View> {
    let (title, lines) = pane.get(|s| s.offer.as_ref().map(|o| install::store_sheet(&o.entry)))?;
    let lines = lines.into_iter().map(|l| text(l).text_size(px(TEXT_SMALL)).text_color(Secure::muted()).into_element());
    Some(
        view().pt_3().child(
            crate::controls::secure_surface(Theme::radius_group())
                .p_3()
                .flex_col()
                .gap_2()
                .child(text(title).font_weight(FontWeight::SEMIBOLD))
                .children(lines)
                .child(view().flex_row().gap_2().child(secure_button("Install", move || pane.confirm_offer(), true)).child(secure_button(
                    "Cancel",
                    move || pane.set(|s| s.offer = None),
                    false,
                ))),
        ),
    )
}

fn choice<H: Host>(pane: Pane<H>) -> Option<View> {
    let (input, assets) = pane.get(|s| s.choice.clone())?;
    let buttons = assets.into_iter().map(|a| {
        let input = input.clone();
        pill(shown(&a.name), move || pane.install(Install::GithubAsset(input.clone(), a.url.clone()), None)).into_element()
    });
    Some(group().child(row().child(label("This release has several packages. Choose one:"))).child(row().children(buttons)))
}

fn github_row<H: Host>(pane: Pane<H>) -> View {
    let busy = pane.get(|s| s.busy);
    let install = move || {
        let input = pane.repo.with_untracked(|r| r.0.trim().to_owned());
        if input.is_empty() {
            return pane.say(true, "Enter a repository.");
        }
        pane.install(Install::Github(input), None);
    };
    let field = input()
        .value(move || pane.repo.with(|r| r.0.clone()))
        .on_change(move |v: &str| pane.repo.set(Repo(v.to_owned())))
        .on_submit(move |_: &str| install())
        .auto_capitalize(AutoCapitalize::None)
        .placeholder("owner/repository")
        .label("GitHub repository")
        .min_w(px(180.))
        .flex_1()
        .px(px(8.))
        .py(px(4.))
        .rounded(px(Theme::radius_row()))
        .bg(Theme::raised())
        .border_1()
        .border_color(Theme::hairline())
        .text_color(Theme::text());
    row()
        .child(
            view()
                .flex_col()
                .w_full()
                .child(label("From GitHub"))
                .child(small("A Tarn extension from a repository's release. Tarn asks GitHub only when you press Install.")),
        )
        .child(field)
        .child(pill(if busy { "Working\u{2026}" } else { "Install" }, install))
}

fn official<H: Host>(pane: Pane<H>, rows: &[InstalledExtension]) -> View {
    if !install::catalog_configured() {
        return group().child(row().child(small("The official catalog is not available in this build.")));
    }
    let listed = pane.get(|s| s.catalog.clone()).map(|c| install::catalog_rows(&c, rows)).unwrap_or_default();
    group()
        .child(
            row()
                .child(
                    view()
                        .flex_col()
                        .flex_1()
                        .child(label("Reviewed and signed by Tarn"))
                        .child(small("Tarn asks GitHub for the list only when you press Show.")),
                )
                .child(pill("Show Official Extensions", move || pane.show_official())),
        )
        .child(
            row()
                .child(
                    view()
                        .flex_col()
                        .flex_1()
                        .child(label("Browse the store"))
                        .child(small("Official extensions with screenshots and search; Install in Tarn asks here first.")),
                )
                .child(pill("Open", move || pane.host.open_link(browser_core::extensions::STORE_URL))),
        )
        .children(listed.into_iter().map(|(id, name, description, version, installed)| {
            row()
                .child(view().flex_col().flex_1().child(label(name)).child(small(format!("{version} · {description}"))))
                .child(if installed { small("Installed").into_element() } else { pill("Install", move || pane.install_official(&id)).into_element() })
                .into_element()
        }))
}

fn body<H: Host>(pane: Pane<H>) -> View {
    let host = pane.host;
    let mut page = view().flex_col();
    if host.profile().is_none() {
        return page
            .child(view().px_1().pt_2().child(small("Extensions never run in a private space or a burner tab. Switch to a profile's space to manage them.")));
    }
    if let Some(notice) = host.notice() {
        page = page.child(view().px_1().pt_2().child(note(notice, true)));
    }
    if let Some((error, message)) = pane.get(|s| s.message.clone()) {
        page = page.child(view().px_1().pt_2().child(note(message, error)));
    }
    if let Some(card) = store_offer(pane) {
        page = page.child(card);
    }
    if let Some(card) = consent(pane) {
        page = page.child(card);
    }
    let rows = pane.rows();
    page = page.child(section("Installed"));
    page = if rows.is_empty() {
        page.child(
            view().px_1().child(small("No extensions yet. An extension runs only on the sites you allow, never in private spaces, burner tabs or Lockdown.")),
        )
    } else {
        page.children(rows.iter().cloned().map(|r| view().pb_2().child(installed(pane, r)).into_element()))
    };
    let every = host.with_store(|s| every_site_setting(s.setting(EVERY_SITE_SETTING).ok().flatten().as_deref()));
    page.child(view().pt_2().child(group().child(switch_row(
        "App and Official extensions use every site they list",
        move || every,
        move || {
            let saved = host.with_store(|s| s.set_setting(EVERY_SITE_SETTING, if every { "off" } else { "on" }).map_err(|e| e.to_string()));
            pane.report(saved, "");
        },
    ))))
    .child(view().px_1().pt_1().child(small("Unverified extensions always ask on each site.")))
    .child(section("Add Extensions"))
    .children(choice(pane))
    .child(
        group().child(github_row(pane)).child(
            row()
                .child(view().flex_col().flex_1().child(label("A package file")).child(small("A .zip or .crx Tarn extension. Nobody reviewed it.")))
                .child(accent_pill("Choose File\u{2026}", move || pane.pick_file())),
        ),
    )
    .child(section("Official Extensions"))
    .child(official(pane, &rows))
}

fn opened<H: Host>(host: H) -> Pane<H> {
    let pane = Pane { host, state: global::<State>(), repo: global::<Repo>() };
    if install::catalog_configured() && pane.state.with_untracked(|s| s.catalog.is_none()) {
        let cached = install::cached_catalog(&host.catalog_dir(), &install::catalog_key());
        pane.set(|s| s.catalog = cached);
    }
    pane
}

pub fn extensions<H: Host>(host: H) -> AnyElement {
    let pane = opened(host);
    dynamic(move || body(pane)).into_element()
}

pub fn store_link<H: Host>(host: H, tab: TabKey, id: &str) -> Result<(), String> {
    let pane = opened(host);
    let (mut slot, busy) = pane.state.with_untracked(|s| (s.offer.clone(), s.busy || s.pending.is_some() || s.choice.is_some()));
    pane.store_check(tab, busy, |context, catalog| install::offer_store_link(&mut slot, tab, id, context, catalog))?;
    pane.set(|s| s.offer = slot);
    Ok(())
}

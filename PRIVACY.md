# Privacy notice

Tarn has no accounts, no telemetry and no servers of its own. Browsing data stays on your device.

Tarn is published by Quavon UG (haftungsbeschränkt), Langbehnstraße 39, 83022 Rosenheim, Germany,
contact@tarnbrowser.com ([imprint](https://tarnbrowser.com/imprint)), the controller under the GDPR
for this notice. Tarn runs on macOS, Windows, Linux, iOS and Android; paths and menus below are
macOS's unless a passage names another client. A summary, with the privacy notice of Tarn's website,
is at https://tarnbrowser.com/privacy.

## What stays on your device

History (and, only if you turn on Settings › Search › Search the text of visited pages, up to 4 KB
of each visited page's text), cookies, cache, open tabs, permissions, downloads, favicons, settings,
how often and when each shortcut tip was shown, your snippets, and the Shield card's counters (tracking parameters removed, pages upgraded to HTTPS, local-network
visits blocked, pages on block lists stopped, pop-ups blocked and requests blocked by the Chromium
engine's filter; in total, per day for the last seven days and per site for the sites protected
most) with its design value are stored only on your Mac, separately for each profile, in
`~/Library/Application Support/Tarn` and in WebKit's per-profile website data stores in
`~/Library/WebKit/de.quavon.tarn`; Chromium's data for the one profile that uses it lives in
`~/Library/Application Support/Tarn/Chromium`. Tarn runs without the App Sandbox, so these folders
are protected by your user account's file permissions and FileVault; Chromium's helper processes and
WebKit's content processes keep their own sandboxes. The database file is readable only by your user
account. The private space keeps everything in memory and writes none of it to disk; on Android,
whose system WebView has no in-memory store, it keeps its data in its own folder in the app's
private storage, deleted when the space closes or, after a crash or force-stop, at the next start. A
burner tab (⌥⇧⌘N) goes further: it gets its own in-memory website data store at Strict, records no
history, site icons or downloads list entries, is never restored, and everything it held is dropped
when it closes.

Importing from another browser (Library › Import) copies its history, bookmarks and, where Tarn can
read them, cookies into the profile you choose, on this device only; nothing is sent. On Windows
cookies come only from Firefox and go into that profile's Chromium cookie file.

Elements you hide with Zap Element are kept per profile and host in the database, and WebKit keeps
them as a compiled content rule list; on Windows, Chromium gets them as a style sheet held in
memory. On Windows the ad and annoyance lists also hide elements through such a style sheet, which
the page can see. Private and burner spaces keep none. Clearing a site's data
removes that site's hidden elements; clearing history or deleting the profile's data removes them all.

Session snapshots (Save Space as Snapshot) keep the addresses, titles and groups of a space's tabs
in the same database, per profile, until you delete them; they hold no page state or cookies. A
private space cannot save one. Space rules ("Always open in…") store a host and the space it opens
in; none can be made from or point to the private space or a burner tab.

Uninstalling Tarn on Windows keeps `%LOCALAPPDATA%\Tarn` unless you answer Yes when asked (a silent uninstall always keeps it) and removes Tarn's registration as a browser.

On Windows a downloaded update installer waits in `%LOCALAPPDATA%\Tarn\updates` until you choose
Restart to Update; the folder is emptied at every launch.

A paused download is listed only while Tarn runs: what WebKit needs to continue it stays in memory,
and the part already saved is removed when you quit before resuming.

Read Aloud in the reader speaks the article with the system's on-device voices on every client;
nothing is sent.

On macOS and Windows, Settings › Privacy › Your data exports a profile's stored data as a JSON file
in your Downloads folder and deletes everything in a profile: spaces, tabs, snapshots, history, site
icons, site settings, Shield counters and WebKit's cookies, cache and storage; deleted rows are
overwritten in the database file. On Linux, Settings › Privacy offers Clear History, Clear Download
List and Clear Website Data; none of them clears the list of recently closed tabs. On iOS and
Android, Clear Browsing Data removes history, the downloads list, closed tabs and website data; in
the private space, website data goes when the space closes. Clearing history also removes kept page
text and the Shield card's list of protected sites. Files you downloaded are not deleted.

## Connections the browser itself makes

Apart from the pages you open, Tarn's shell makes only these requests. They are listed in
Settings › Privacy › Connections Tarn makes on macOS and Windows.

| Connection | Receiver | What they see | Status |
|---|---|---|---|
| Update check | GitHub (Releases of this repository) | Your IP address and Tarn version (on Windows only your IP address: Tarn fetches the signed `windows-update.json` and, when it names a later version, downloads that installer); from the `nightly` release in place of the latest one while Settings › General › Nightly builds is on | Once a day while Settings › General › Check for updates automatically is on (the default; on Windows an hour after a check that failed), and when you choose Tarn › Check for Updates… or Settings › General › Check Now, on macOS and Windows |
| Filter-list update | GitHub (Releases of this repository) | Your IP address. Every list is downloaded whatever you switched on, so the download does not reveal your blocking choices | Once a day, on macOS, Windows and Linux; switch off in Settings › Privacy |
| Fraudulent-website check | Apple, which relays Google Safe Browsing; done by WebKit itself, on macOS and iOS. On Android, Google directly, done by the system WebView's Safe Browsing | Shortened hashes of the addresses you visit, not the addresses | While "Block dangerous sites" is on (the default); off with that switch. On iOS and Android the web view's default, which Tarn leaves on and no switch turns off. Windows and Linux make no such check |
| Extension catalog and downloads | GitHub (the repository you name, or the official catalog in `Quavon-dev/tarn-extensions`, not offered yet because its signing key is not set) | Your IP address and which repository or extension you install or check | Only when you click Install, Check for Updates or Show Official Extensions in Settings › Extensions; an Install in Tarn link from extensions.tarnbrowser.com makes no request until you click Install on the confirmation it opens there; switch off in Settings › Privacy (installing from a file keeps working) |
| Theme catalog and downloads | GitHub (the repository you name, or the official catalog in `Quavon-dev/tarn-themes`, not offered yet because its signing key is not set) | Your IP address and which repository or theme you install or check | Only when you click Install, Check for Updates or Show Official Themes in Settings › Appearance, on every client; on macOS and Windows switch off with the same Settings › Privacy switch as extensions; Linux, iOS and Android have no such switch yet (themes from a file or the clipboard keep working) |
| Site icon | The site you are on, or a server it already loaded from (such as its CDN, without cookies), through the tab's web engine with your Shield rules | Same as visiting the site | After a page loads over https; never under Lockdown; cleared with history |
| Translation (builds with the `translation` feature) | Nobody: Apple's Translation framework runs on your Mac | Nothing. Only language packs you installed in System Settings › General › Language & Region › Translation Languages are used; Tarn never downloads one | Only when you click Translate on the banner; never under Lockdown |
| AI request (only if you enable AI) | Anthropic, or Ollama on your own Mac | The page content you choose to send | Not built yet |

## Chromium tabs

On Windows every tab uses Chromium, and its data lives in `%LOCALAPPDATA%\Tarn\Chromium`. On macOS
WebKit is the default engine. You can open a tab in Chromium instead, or make Chromium the default
in Settings › General, while automatic updates are on: Chromium's security fixes reach you only
through them, and Chromium tabs close when this build's Chromium is 28 days old or no signed update
check has succeeded for seven days, or none yet on a new install. Before the first Chromium tab,
Tarn tells you that Chromium connects to Google. When a profile uses Chromium:

- Chromium's data (cookies, cache, storage) lives in `~/Library/Application Support/Tarn/Chromium`,
  for one profile only; deleting that profile's website data or the profile deletes it, and private
  and other profiles never use Chromium.
- Chromium itself writes `~/Library/Application Support/Google/RLZ/RlzStore.plist` (shared with
  Google Chrome, so Tarn leaves it alone) and a `Tarn Safe Storage` item in your login keychain,
  which encrypts its cookies.
- Logins carry over between WebKit and Chromium for sites you open in a Chromium tab yourself:
  the cookie bridge copies those sites' cookies in memory between the two engines of the same
  profile; nothing is written anywhere else, and cross-site (`SameSite=None`) cookies, which
  include partitioned ones, never move from Chromium to WebKit.
- Chromium sends its own unmodified Chrome user agent, so sites see Chrome rather than Safari.
- Chromium's background networking, component updater and search-engine choice screen are off,
  third-party cookies are blocked, and every permission that would ask (location, camera, microphone, notifications, devices and others) defaults to "block". The list of hosts
  Chromium may contact on its own is not enforced.

## Blocking

On macOS and Windows, ad, tracker, cookie-banner, malware, phishing, adult-content and pop-up
blocking runs on your device against lists in the signed bundle above: EasyList, EasyPrivacy,
EasyList Cookie, AdGuard Base, uBlock Origin's filters, URLhaus, CERT Polska, the Scam Blocklist,
the Global Anti-Scam Org list, the stalkerware indicators, HaGeZi's threat, adult and pop-up lists
and OISD's adult list. `licenses.txt` in the bundle names each source and its licence. Linux, iOS
and Android do not download these lists. Which sites you visit, which ones were blocked and which
ones you let through are not sent anywhere; apart from the Shield counters above, which name the
page a block happened on, they are not logged. Ad and tracker blocks in WebKit tabs are not counted,
because WebKit does not report what its blocking rules stop. "Visit Anyway" is remembered only until
you quit.

## Search

The default search engine is DuckDuckGo. What you type in the command bar is sent to DuckDuckGo only
when you press Enter on a search; search suggestions are off, so keystrokes are not sent while you
type. No affiliate or tracking parameters are added. With Settings › Search › Search shortcuts on
(off by default), a `!key` such as `!yt` sends what you type to the site it names instead of your
search engine, in every space including the private space, again only when you press Enter.

## Crash reports

Crash reports are kept locally. Nothing is sent unless you send it yourself. On macOS and Windows,
when Tarn stops on an internal error, it writes where in its code that happened and a backtrace to
`last-panic.txt` in its data folder, replacing the previous one; error messages that could hold a
page address are left out.

## Legal basis, recipients and your rights

Quavon receives none of the data above: what Tarn stores stays on your device, and Tarn sends
nothing to Quavon. Keeping it on your device is needed for the browser to do what you ask (section
25(2) no. 2 TDDDG); it is kept until you delete it. Uninstalling Tarn on macOS or Linux leaves its
data folders in place, and on Windows the uninstaller deletes `%LOCALAPPDATA%\Tarn` only if you
answer Yes (a silent uninstall always keeps it); on iOS and Android the system deletes the data with
the app.

The connections in the table go to the receivers named there, who process your IP address under
their own privacy terms. Update and filter-list checks keep Tarn secure and its block lists current,
a legitimate interest (Art. 6(1)(f) GDPR); you can switch them off as described. Catalog and
download requests happen because you ask for them (Art. 6(1)(b) GDPR), and so do Chromium's own
requests to Google, because you chose Chromium (on Windows, because you use Tarn). The
fraudulent-website check protects you from phishing and malware, a legitimate interest
(Art. 6(1)(f) GDPR). GitHub, Apple and Google
may process this data in the United States, which the EU–US Data Privacy Framework covers for
companies certified under it. Apple's App Store and Google Play process your store account and
download under their own terms; Quavon receives only aggregated statistics from them.

If you write to contact@tarnbrowser.com, Quavon uses your message only to answer it (Art. 6(1)(b) or
(f) GDPR) and deletes it when the matter is settled, unless the law requires keeping it. Issues and
pull requests on GitHub are public: do not put personal data or privacy requests there.

You have the right of access, rectification, erasure, restriction, data portability and to object to
processing based on legitimate interest (Art. 15 to 21 GDPR); write to contact@tarnbrowser.com. You
may complain to a data protection authority; the one responsible for Quavon is the Bayerisches
Landesamt für Datenschutzaufsicht (BayLDA), Promenade 18, 91522 Ansbach, Germany. Tarn makes no
automated decisions about you and needs no data from you to work.

## Contact

Questions about this notice: contact@tarnbrowser.com.

This notice is not legal advice; it describes how the software behaves.

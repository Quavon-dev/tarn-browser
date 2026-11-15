# Trademark policy

"Tarn", the Tarn logo (`crates/ui/brand/mark.svg`, `mobile/app/assets/brand/mark.svg`) and the Tarn
app icon (`packaging/Tarn.icns`, `mobile/app/assets/brand/icon.png`,
`mobile/android/host/src/main/res/drawable-nodpi/ic_launcher_art.png` and the icon
`linux/packaging/install.sh` installs from the logo) are trademarks of Quavon UG
(haftungsbeschränkt). The source code license ([`LICENSE`](LICENSE)) covers the code, not these
marks: as section 7(e) of the GNU General Public License version 3 allows, it grants no rights under
trademark law.

## Allowed without asking

- Building and running Tarn from this repository for yourself or within your organisation.
- Passing on unmodified official builds downloaded from this repository's releases.
- Packaging unmodified source of a tagged release for a Linux or BSD distribution under the Tarn
  name, outside app stores.
- Saying truthfully that your project is "based on Tarn" or "a fork of Tarn", or referring to Tarn
  in articles, reviews, tutorials and comparisons.

## Not allowed without written permission

- Distributing a modified version, or a build you made yourself other than as allowed above, under
  the name "Tarn" or with the Tarn logo or app icon.
- Publishing any version of Tarn, modified or not, in an app store or similar platform under the
  Tarn name, logo or app icon.
- Using the Tarn name or logo for another product, a domain or an account in a way that suggests it
  comes from or is endorsed by Quavon UG (haftungsbeschränkt).

## For forks

A fork needs a name of its own and its own logo and icon instead of the files above. These further
steps are not licence conditions, but they keep users and update systems from mistaking a fork for
Tarn: change the bundle and application identifiers (`dev.tarn.*`), the update feed and its key in
`packaging/updates.conf`, and the repository links in `Cargo.toml`, `linux/packaging/` and
`scripts/appcast.sh`.

To ask for permission, open an issue in this repository.

#!/usr/bin/env bash
# The signing Mac's half of the release pipeline (docs/release.md). Installed by hand from a reviewed commit,
# never updated by the pipeline, run by a dedicated account. It executes nothing it fetches: git
# objects, release assets, CEF and Sparkle archives are data. Keys are read from that account's
# keychain at run time only. Setup and configuration: docs/release.md.
#
#   sign-release.sh --config FILE [--skip-soak] [--acknowledge-single-source] [TAG]
#
# Without TAG it handles every draft release. --skip-soak is for an exploited Chromium bug only.
# --acknowledge-single-source signs one TAG without the config's second_route, after the CEF
# version is typed on stdin.
set -euo pipefail
umask 077

readonly CEF_HOST=cef-builds.spotifycdn.com
readonly SPARKLE_HOST=github.com
readonly PREDICATE_TYPE=https://github.com/Quavon-dev/tarn-browser/attestations/release/v1
readonly HISTORY=https://versionhistory.googleapis.com/v1/chrome/platforms/mac/channels/all/versions/all/releases
readonly SOAK_SECONDS=86400
readonly FRAMEWORK="Contents/Frameworks/Chromium Embedded Framework.framework"
readonly SPARKLE=Contents/Frameworks/Sparkle.framework
readonly HELPERS=("" " (GPU)" " (Renderer)" " (Plugin)" " (Alerts)")
readonly NOTICES=(LICENSE.txt Chromium-CREDITS.html CEF-LICENSE.txt Third-Party-Licenses.txt)
readonly VERSION_RE='^[0-9]+\.[0-9]+\.[0-9]+$'
readonly CHROMIUM_RE='^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$'

refuse() { echo "refused: $*" >&2; exit 1; }

field() {
  local value
  value="$(grep -E "^$2=" "$1" | head -n1 | cut -d= -f2-)"
  [[ -n "$value" ]] || refuse "$1 has no $2"
  printf '%s' "$value"
}

newer() { [[ "$1" != "$2" && "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n1)" == "$1" ]]; }

check_lock() {
  local lock="$1" today="$2" cef chromium date
  [[ "$(cut -d= -f1 "$lock" | LC_ALL=C sort | tr '\n' ' ')" == "cef_version chromium_version host release_date sha256_macosarm64 sha256_macosx64 sha256_windows64 " ]] \
    || refuse "cef.lock has other keys than the seven expected"
  cef="$(field "$lock" cef_version)"
  chromium="$(field "$lock" chromium_version)"
  date="$(field "$lock" release_date)"
  [[ "$(field "$lock" host)" == "$CEF_HOST" ]] || refuse "cef.lock names another download host"
  [[ "$cef" =~ ^[0-9]+\.[0-9]+\.[0-9]+\+g[0-9a-f]{7}\+chromium-[0-9.]+$ && "$cef" == *"+chromium-$chromium" ]] \
    || refuse "cef.lock cef_version $cef does not carry chromium_version $chromium"
  [[ "$chromium" =~ $CHROMIUM_RE ]] || refuse "bad chromium_version"
  [[ "$(field "$lock" sha256_macosarm64)" =~ ^[0-9a-f]{64}$ && "$(field "$lock" sha256_macosx64)" =~ ^[0-9a-f]{64}$ && "$(field "$lock" sha256_windows64)" =~ ^[0-9a-f]{64}$ ]] \
    || refuse "bad sha256 in cef.lock"
  [[ "$date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || refuse "bad release_date"
  [[ ! "$date" > "$today" ]] || refuse "release date $date is in the future"
}

check_sparkle_lock() {
  local lock="$1"
  [[ "$(cut -d= -f1 "$lock" | LC_ALL=C sort | tr '\n' ' ')" == "host release_date sha256 sparkle_version " ]] \
    || refuse "sparkle.lock has other keys than the four expected"
  [[ "$(field "$lock" host)" == "$SPARKLE_HOST" ]] || refuse "sparkle.lock names another download host"
  [[ "$(field "$lock" sparkle_version)" =~ ^2\.[0-9]+\.[0-9]+$ ]] || refuse "bad sparkle_version"
  [[ "$(field "$lock" release_date)" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || refuse "bad release_date in sparkle.lock"
  [[ "$(field "$lock" sha256)" =~ ^[0-9a-f]{64}$ ]] || refuse "bad sha256 in sparkle.lock"
}

check_google() {
  local json="$1" chromium="$2" date="$3" published
  published="$(jq -r --arg v "$chromium" '[.releases[]? | select(.version == $v and (.name | test("^chrome/platforms/mac/channels/(stable|extended)/")))
    | .serving.startTime[:10]] | min // empty' "$json")"
  [[ -n "$published" ]] || refuse "Chromium $chromium is not on Google's stable list"
  [[ "$published" == "$date" ]] || refuse "Google published Chromium $chromium on $published, the lock says $date"
}

check_versions() {
  local old_app="$1" new_app="$2" old_chromium="$3" new_chromium="$4"
  [[ "$new_app" =~ $VERSION_RE ]] || refuse "bad app version $new_app"
  newer "$new_app" "$old_app" || refuse "app version $new_app is not higher than $old_app"
  [[ "$new_chromium" == "$old_chromium" ]] || newer "$new_chromium" "$old_chromium" \
    || refuse "Chromium $new_chromium is older than $old_chromium"
}

check_ancestry() {
  git -C "$1" merge-base --is-ancestor "$2" refs/remotes/origin/main || refuse "commit $2 is not on main"
}

bump_members() {
  awk -v old="version = \"$1\"" -v new="version = \"$2\"" 'BEGIN { RS = ""; ORS = "\n\n" }
    $0 !~ /\nsource = / { n = split($0, line, "\n"); $0 = line[1]; for (i = 2; i <= n; i++) $0 = $0 "\n" (line[i] == old ? new : line[i]) }
    { print }'
}

check_diff() {
  local mirror="$1" old="$2" new="$3" old_app="$4" new_app="$5" files
  files="$(git -C "$mirror" diff --no-ext-diff --no-textconv --no-renames --name-only "$old" "$new")"
  [[ -z "$(grep -vxE 'packaging/cef\.lock|Cargo\.toml|Cargo\.lock' <<<"$files")" ]] \
    || refuse "the diff since the last signed release touches more than the engine lock and the version"
  [[ "$(git -C "$mirror" show "$old:Cargo.toml" | sed "/^\[workspace.package\]/,/^\[/s/^version = \"${old_app//./\\.}\"$/version = \"$new_app\"/")" \
    == "$(git -C "$mirror" show "$new:Cargo.toml")" ]] || refuse "Cargo.toml changes more than the workspace version"
  [[ "$(git -C "$mirror" show "$old:Cargo.lock" | bump_members "$old_app" "$new_app")" \
    == "$(git -C "$mirror" show "$new:Cargo.lock" | bump_members "$new_app" "$new_app")" ]] \
    || refuse "Cargo.lock changes more than the versions of the workspace crates"
}

maintainer_signed() {
  local out
  out="$(git -C "$1" -c gpg.ssh.allowedSignersFile="$3" -c gpg.openpgp.program=false -c gpg.x509.program=false \
    verify-tag "$2" 2>&1)" || return 1
  [[ "$out" == *'Good "git" signature for '* ]]
}

# The workflow files that ran must be the ones at the tag: a run from a later commit on main could
# have built something else and named this commit in its predicate.
check_attestation() {
  local json="$1" digest="$2" commit="$3" repo="$4" tag="$5" signed="$6" mirror="$7" workflows signer config
  workflows="$(git -C "$mirror" rev-parse --verify "$commit:.github/workflows")"
  while read -r signer config; do
    [[ "$signer" =~ ^[0-9a-f]{40}$ && "$config" =~ ^[0-9a-f]{40}$ ]] || continue
    [[ "$(git -C "$mirror" rev-parse --verify -q "$signer:.github/workflows" || true)" == "$workflows" \
      && "$(git -C "$mirror" rev-parse --verify -q "$config:.github/workflows" || true)" == "$workflows" ]] && return 0
  done < <(jq -r --arg digest "$digest" --arg commit "$commit" --arg repo "https://github.com/$repo" --arg tag "$tag" \
    --arg type "$PREDICATE_TYPE" --argjson signed "$signed" '
    .[] | .verificationResult as $r | $r.signature.certificate as $c
    | select(($r.statement.subject | any(.digest.sha256 == $digest))
      and $r.statement.predicateType == $type
      and $r.statement.predicate.commit == $commit
      and $r.statement.predicate.tag == $tag
      and $c.runnerEnvironment == "github-hosted"
      and $c.sourceRepositoryURI == $repo
      and (($c.buildSignerURI == "\($repo)/.github/workflows/release.yml@refs/heads/main"
            and $c.buildConfigURI == "\($repo)/.github/workflows/chromium-update.yml@refs/heads/main"
            and $c.sourceRepositoryRef == "refs/heads/main")
        or ($signed
            and $c.buildSignerURI == "\($repo)/.github/workflows/release.yml@refs/tags/\($tag)"
            and $c.buildConfigURI == $c.buildSignerURI
            and $c.sourceRepositoryRef == "refs/tags/\($tag)"
            and $c.sourceRepositoryDigest == $commit)))
    | "\($c.buildSignerDigest) \($c.buildConfigDigest)"' "$json")
  refuse "no attestation of $digest from release.yml on main, called by chromium-update.yml on a GitHub-hosted runner with the tag's workflow files, for commit $commit"
}

check_soak() {
  local state="$1" sha256="$2" now="$3" seen
  [[ "$sha256" =~ ^[0-9a-f]{64}$ ]] || refuse "bad sha256"
  mkdir -p "$state/seen"
  [[ -f "$state/seen/$sha256" ]] || echo "$now" > "$state/seen/$sha256"
  seen="$(cat "$state/seen/$sha256")"
  (( now - seen >= SOAK_SECONDS )) || refuse "CEF build $sha256 first seen $(( (now - seen) / 3600 )) hours ago; it soaks for 24"
}

# A second CEF download over another route, so one network or CDN response alone cannot pick the
# archive: "proxy https://HOST[:PORT]" tunnels to the CDN through that proxy, "mirror https://HOST[/PATH]"
# fetches the same file name from another host, which must not end up at the CDN.
host_of() {
  local host="${1#*://}"
  host="${host%%/*}"
  host="${host##*@}"
  host="${host%%:*}"
  tr '[:upper:]' '[:lower:]' <<<"${host%.}"
}

second_route() {
  local config="$1" single_source="$2" route
  route="$(sed -n 's/^second_route=//p' "$config" | head -n1)"
  if [[ -z "$route" ]]; then
    [[ "$single_source" == 1 ]] || refuse "$config has no second_route (docs/release.md); --acknowledge-single-source with a TAG signs without one"
    return 0
  fi
  [[ "$route" =~ ^(proxy|mirror)\ https://[A-Za-z0-9.-]+(:[0-9]+)?(/[A-Za-z0-9._~/-]*)?$ ]] || refuse "bad second_route $route"
  [[ "$route" != mirror* || "$(host_of "${route#mirror }")" != "$CEF_HOST" ]] || refuse "second_route mirrors $CEF_HOST itself"
  printf '%s' "${route%/}"
}

fetch_second() {
  local route="$1" file="${2//+/%2B}.tar.bz2" out="$3" landed
  if [[ "$route" == proxy* ]]; then
    fetch --proxy "${route#proxy }" --noproxy "" -o "$out" "https://$CEF_HOST/$file"
  else
    landed="$(fetch -w '%{url_effective}' -o "$out" "${route#mirror }/$file")"
    [[ "$(host_of "$landed")" != "$CEF_HOST" ]] || refuse "second_route's mirror redirected to $CEF_HOST"
  fi
}

same_download() {
  [[ "$(shasum -a 256 "$1" | cut -d' ' -f1)" == "$(shasum -a 256 "$2" | cut -d' ' -f1)" ]] \
    || refuse "the CEF download over second_route differs from the direct one"
}

acknowledge_single_source() {
  local state="$1" tag="$2" cef="$3" typed
  echo "WARNING: $tag's CEF $cef is checked against a single download from $CEF_HOST (--acknowledge-single-source)" >&2
  printf 'Type the CEF version to sign with it: ' >&2
  IFS= read -r typed || refuse "no acknowledgement on stdin"
  [[ "$typed" == "$cef" ]] || refuse "the typed version is not $cef"
  mkdir -p "$state"
  echo "$(date -u +%FT%TZ) $tag CEF $cef: single-source signing acknowledged by $(id -un)" >> "$state/single-source.log"
}

manifest() {
  (cd "$1" && find . -mindepth 1 | LC_ALL=C sort | while IFS= read -r path; do
    if [[ -L "$path" ]]; then echo "l $(readlink "$path") $path"
    elif [[ -d "$path" ]]; then echo "d $path"; else echo "f $(stat -f %Lp "$path") $(shasum -a 256 < "$path" | cut -d' ' -f1) $path"; fi
  done)
}

# The same steps as scripts/bundle.sh after it unpacks Sparkle.
sparkle_reference() {
  mkdir "$2"
  ditto "$1/Sparkle.framework" "$2/Sparkle.framework"
  rm -rf "$2/Sparkle.framework/Versions/B/XPCServices/Downloader.xpc"
  ditto "$1/LICENSE" "$2/LICENSE"
}

update_keys() { plutil -convert json -o - "$1" | jq -c 'with_entries(select(.key | startswith("SU")))'; }

check_tree() {
  local app="$1" reference="$2" sparkle="$3" template="$4" version="$5" feed="$6" key="$7" notices="$8" flavor helper expected info notice
  # Sparkle's versioned framework is made of symlinks; they must equal those of this Mac's download.
  [[ -z "$(find "$app" "$reference" "$sparkle" \( ! -type f ! -type d ! \( -type l \( -path "$app/$SPARKLE/*" -o -path "$sparkle/Sparkle.framework/*" \) \) \) \
    -o -name "*[[:cntrl:]]*" -o -lname "*[[:cntrl:]]*" -o -perm -4000 -o -perm -2000 | head -n1)" ]] \
    || refuse "the bundle holds a symlink outside Sparkle, a special file, a set-id file or a control character in a name or link"
  expected="$(
    printf '%s\n' Contents Contents/Info.plist Contents/MacOS Contents/MacOS/tarn Contents/Resources \
      Contents/Resources/Tarn.icns Contents/Resources/Sparkle-LICENSE.txt Contents/Frameworks "$FRAMEWORK" "$SPARKLE"
    for notice in "${NOTICES[@]}"; do echo "Contents/Resources/$notice"; done
    for flavor in "${HELPERS[@]}"; do
      helper="Contents/Frameworks/tarn Helper$flavor.app"
      printf '%s\n' "$helper" "$helper/Contents" "$helper/Contents/Info.plist" "$helper/Contents/MacOS" \
        "$helper/Contents/MacOS/tarn Helper$flavor"
    done
  )"
  [[ "$(cd "$app" && find . -mindepth 1 ! -path "./$FRAMEWORK/*" ! -path "./$SPARKLE/*" | sed 's|^\./||' | LC_ALL=C sort)" == "$(LC_ALL=C sort <<<"$expected")" ]] \
    || refuse "the bundle holds other files than the app, the two frameworks and five helpers"
  [[ -d "$app/$FRAMEWORK" && "$(manifest "$app/$FRAMEWORK")" == "$(manifest "$reference")" ]] \
    || refuse "the framework differs from this Mac's own CEF download"
  [[ -d "$app/$SPARKLE" && "$(manifest "$app/$SPARKLE")" == "$(manifest "$sparkle/Sparkle.framework")" ]] \
    || refuse "Sparkle differs from this Mac's own Sparkle download"
  cmp -s "$app/Contents/Resources/Sparkle-LICENSE.txt" "$sparkle/LICENSE" || refuse "Sparkle's licence differs from this Mac's own download"
  for notice in "${NOTICES[@]}"; do
    cmp -s "$app/Contents/Resources/$notice" "$notices/$notice" || refuse "$notice differs from this Mac's own CEF download or the tag's packaging"
  done
  info="$(plutil -convert json -o - "$app/Contents/Info.plist" | jq -cS .)"
  expected="$(sed "s/@VERSION@/$version/g" "$template" | plutil -convert json -o - - \
    | jq -cS --arg feed "$feed" --arg key "$key" '. + {SUFeedURL: $feed, SUPublicEDKey: $key}')"
  [[ "$info" == "$expected" ]] || refuse "Info.plist differs from packaging/Info.plist with version $version, the pinned feed and public key"
  for flavor in "${HELPERS[@]}"; do
    [[ "$(update_keys "$app/Contents/Frameworks/tarn Helper$flavor.app/Contents/Info.plist")" == "{}" ]] \
      || refuse "helper$flavor carries update keys"
  done
}

work=""
scratch=""
unpack_user=""
# A refused bundle can carry deny-delete ACLs; clear them without following its symlinks.
cleanup() {
  [[ -z "$work" ]] || { find "$work" -exec chmod -h -N {} +; rm -rf "$work"; }
  [[ -z "$scratch" ]] || { sudo -n -u "$unpack_user" find "$scratch" -exec chmod -h -N {} +; sudo -n -u "$unpack_user" rm -rf "$scratch"; }
}

fetch() { curl -q --proto '=https' --tlsv1.2 -sSfL --max-time 900 "$@"; }

unpack() (
  umask 022
  local user="$1" archive="$2" dest="$3" kind="$4"
  sudo -n -u "$user" mkdir -m 755 "$dest"
  dd if="$archive" status=none | sudo -n -u "$user" dd of="$dest.in" status=none
  if [[ "$kind" == zip ]]; then
    sudo -n -u "$user" ditto -x -k "$dest.in" "$dest"
  else
    sudo -n -u "$user" tar -xf "$dest.in" -C "$dest" --no-same-owner "${@:5}"
  fi
)

sign_tag() {
  local config="$1" tag="$2" skip_soak="$3" single_source="$4"
  local repo mirror state signers jit media identity notary feed key route
  repo="$(field "$config" repo)"
  mirror="$(field "$config" mirror)"
  state="$(field "$config" state)"
  signers="$(field "$config" allowed_signers)"
  jit="$(field "$config" jit_entitlements)"
  media="$(field "$config" media_entitlements)"
  identity="$(field "$config" identity)"
  notary="$(field "$config" notary_profile)"
  unpack_user="$(field "$config" unpack_user)"
  feed="$(field "$config" feed_url)"
  key="$(field "$config" public_ed_key)"
  route="$(second_route "$config" "$single_source")"
  [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || refuse "bad tag $tag"
  [[ ! -e "$state/signed/$tag" ]] || { echo "$tag: already signed"; return 0; }

  local commit signed=false lock app_version chromium cef sha256
  commit="$(git -C "$mirror" rev-parse --verify "refs/tags/$tag^{commit}")"
  check_ancestry "$mirror" "$commit"
  if maintainer_signed "$mirror" "$tag" "$signers"; then signed=true; fi
  sudo -n -u "$unpack_user" true
  work="$(mktemp -d)"
  trap cleanup EXIT
  git -C "$mirror" show "$commit:packaging/cef.lock" > "$work/cef.lock"
  git -C "$mirror" show "$commit:packaging/sparkle.lock" > "$work/sparkle.lock"
  git -C "$mirror" show "$commit:packaging/Info.plist" > "$work/Info.plist"
  lock="$work/cef.lock"
  check_lock "$lock" "$(date -u +%F)"
  check_sparkle_lock "$work/sparkle.lock"
  app_version="$(git -C "$mirror" show "$commit:Cargo.toml" | sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p')"
  [[ "$tag" == "v$app_version" ]] || refuse "tag $tag does not name the app version $app_version"
  chromium="$(field "$lock" chromium_version)"
  cef="$(field "$lock" cef_version)"
  sha256="$(field "$lock" sha256_macosarm64)"
  if [[ -f "$state/last" ]]; then
    check_versions "$(field "$state/last" app_version)" "$app_version" "$(field "$state/last" chromium_version)" "$chromium"
    [[ "$signed" == true ]] || check_diff "$mirror" "$(field "$state/last" commit)" "$commit" "$(field "$state/last" app_version)" "$app_version"
  else
    [[ "$signed" == true ]] || refuse "nothing signed yet: the first release needs a maintainer-signed tag"
  fi
  fetch -o "$work/google.json" "$HISTORY?filter=version=$chromium"
  check_google "$work/google.json" "$chromium" "$(field "$lock" release_date)"
  [[ "$skip_soak" == 1 ]] || check_soak "$state" "$sha256" "$(date +%s)"
  [[ -n "$route" ]] || acknowledge_single_source "$state" "$tag" "$cef"

  local name="cef_binary_${cef}_macosarm64_minimal" zip="$work/Tarn-$tag-unsigned.zip" digest
  fetch --noproxy '*' -o "$work/cef.tar.bz2" "https://$CEF_HOST/${name//+/%2B}.tar.bz2"
  [[ "$(shasum -a 256 "$work/cef.tar.bz2" | cut -d' ' -f1)" == "$sha256" ]] || refuse "this Mac's CEF download does not have the lock's SHA-256"
  if [[ -n "$route" ]]; then
    fetch_second "$route" "$name" "$work/cef-second.tar.bz2"
    same_download "$work/cef.tar.bz2" "$work/cef-second.tar.bz2"
  fi
  local sparkle_version
  sparkle_version="$(field "$work/sparkle.lock" sparkle_version)"
  fetch -o "$work/sparkle.tar.xz" "https://$SPARKLE_HOST/sparkle-project/Sparkle/releases/download/$sparkle_version/Sparkle-$sparkle_version.tar.xz"
  [[ "$(shasum -a 256 "$work/sparkle.tar.xz" | cut -d' ' -f1)" == "$(field "$work/sparkle.lock" sha256)" ]] \
    || refuse "this Mac's Sparkle download does not have the lock's SHA-256"
  gh release download "$tag" --repo "$repo" --pattern "Tarn-$tag-unsigned.zip" --dir "$work"
  digest="$(shasum -a 256 "$zip" | cut -d' ' -f1)"
  gh attestation verify "$zip" --repo "$repo" --predicate-type "$PREDICATE_TYPE" --format json > "$work/attestation.json"
  check_attestation "$work/attestation.json" "$digest" "$commit" "$repo" "$tag" "$signed" "$mirror"

  local app
  scratch="$(sudo -n -u "$unpack_user" mktemp -d)"
  sudo -n -u "$unpack_user" chmod 755 "$scratch"
  unpack "$unpack_user" "$zip" "$scratch/app" zip
  unpack "$unpack_user" "$work/cef.tar.bz2" "$scratch/cef" tar
  unpack "$unpack_user" "$work/sparkle.tar.xz" "$scratch/sparkle" tar Sparkle.framework LICENSE
  [[ "$(ls -A "$scratch/app")" == Tarn.app ]] || refuse "the archive holds more than Tarn.app"
  ditto "$scratch/app/Tarn.app" "$work/Tarn.app"
  ditto "$scratch/cef/$name/Release/Chromium Embedded Framework.framework" "$work/reference.framework"
  sparkle_reference "$scratch/sparkle" "$work/sparkle"
  app="$work/Tarn.app"
  mkdir "$work/notices"
  ditto "$scratch/cef/$name/CREDITS.html" "$work/notices/Chromium-CREDITS.html"
  ditto "$scratch/cef/$name/LICENSE.txt" "$work/notices/CEF-LICENSE.txt"
  git -C "$mirror" show "$commit:packaging/third-party-licenses.txt" > "$work/notices/Third-Party-Licenses.txt" 2>/dev/null \
    || refuse "the tag has no packaging/third-party-licenses.txt"
  git -C "$mirror" show "$commit:LICENSE" > "$work/notices/LICENSE.txt" 2>/dev/null || refuse "the tag has no LICENSE"
  check_tree "$app" "$work/reference.framework" "$work/sparkle" "$work/Info.plist" "$app_version" "$feed" "$key" "$work/notices"
  # The manifest does not cover ACLs or extended attributes, and ditto would ship them.
  find "$app" -exec chmod -h -N {} +
  xattr -crs "$app"

  sign() { codesign --force --options runtime --timestamp -s "$identity" "$@"; }
  sign "$app/$FRAMEWORK/Libraries/"*.dylib
  sign "$app/$FRAMEWORK"
  # Sparkle's order for Developer ID, inside out; the downloader service is not shipped.
  sign "$app/$SPARKLE/Versions/B/XPCServices/Installer.xpc" "$app/$SPARKLE/Versions/B/Autoupdate" "$app/$SPARKLE/Versions/B/Updater.app"
  sign "$app/$SPARKLE"
  local flavor helper
  for flavor in "${HELPERS[@]}"; do
    helper="$app/Contents/Frameworks/tarn Helper$flavor.app"
    if [[ "$flavor" == " (GPU)" || "$flavor" == " (Renderer)" ]]; then sign --entitlements "$jit" "$helper"
    elif [[ -z "$flavor" ]]; then sign --entitlements "$media" "$helper"
    else sign "$helper"; fi
  done
  sign --entitlements "$media" "$app"
  codesign --verify --deep --strict "$app"
  ditto -c -k --keepParent "$app" "$work/notarize.zip"
  xcrun notarytool submit "$work/notarize.zip" --keychain-profile "$notary" --wait --output-format json > "$work/notary.json"
  jq -e '.status == "Accepted"' "$work/notary.json" >/dev/null || refuse "notarisation: $(jq -c . "$work/notary.json")"
  xcrun stapler staple "$app"
  spctl --assess --type execute "$app"
  ditto -c -k --keepParent "$app" "$work/Tarn-$tag.zip"
  local dmg="$work/Tarn-$tag.dmg"
  mkdir "$work/dmg"
  ditto "$app" "$work/dmg/Tarn.app"
  ln -s /Applications "$work/dmg/Applications"
  hdiutil create -quiet -volname Tarn -srcfolder "$work/dmg" -format UDZO -fs HFS+ "$dmg"
  codesign --force --timestamp -s "$identity" "$dmg"
  xcrun notarytool submit "$dmg" --keychain-profile "$notary" --wait --output-format json > "$work/notary-dmg.json"
  jq -e '.status == "Accepted"' "$work/notary-dmg.json" >/dev/null || refuse "notarisation of the disk image: $(jq -c . "$work/notary-dmg.json")"
  xcrun stapler staple "$dmg"
  local assessment
  assessment="$(spctl -a -vv -t open --context context:primary-signature "$dmg" 2>&1)" && [[ "$assessment" == *"source=Notarized Developer ID"* ]] \
    || refuse "Gatekeeper does not accept the disk image as notarised"
  gh release upload "$tag" "$work/Tarn-$tag.zip" "$dmg" --repo "$repo" --clobber

  mkdir -p "$state/signed"
  printf '%s\n' "tag=$tag" "commit=$commit" "app_version=$app_version" "chromium_version=$chromium" > "$state/last"
  cp "$state/last" "$state/signed/$tag"
  echo "$tag: signed, notarised and uploaded"
}

main() {
  local config="" skip_soak=0 single_source=0 tag="" repo mirror status=0 drafts draft args
  while (($#)); do
    case "$1" in
      --config) config="${2:-}"; shift 2 ;;
      --skip-soak) skip_soak=1; shift ;;
      --acknowledge-single-source) single_source=1; shift ;;
      -*) refuse "unknown option $1" ;;
      *) [[ -z "$tag" ]] || refuse "one tag at a time"; tag="$1"; shift ;;
    esac
  done
  [[ -n "$config" ]] || refuse "usage: sign-release.sh --config FILE [--skip-soak] [--acknowledge-single-source] [TAG]"
  [[ "$single_source" == 0 || -n "$tag" ]] || refuse "--acknowledge-single-source signs one TAG at a time"
  [[ -f "$config" && -O "$config" && -z "$(find "$config" -perm +022)" ]] || refuse "$config must be a file owned by this user and writable only by it"
  [[ "$(id -u)" != 0 ]] || refuse "runs as the signing account, not as root"
  repo="$(field "$config" repo)"
  mirror="$(field "$config" mirror)"
  [[ "$repo" =~ ^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$ ]] || refuse "bad repo $repo"
  GH_TOKEN="$(security find-generic-password -s "$(field "$config" token_service)" -w)"
  export GH_TOKEN
  if [[ -n "$tag" ]]; then
    git -C "$mirror" fetch --quiet --prune origin '+refs/heads/main:refs/remotes/origin/main' '+refs/tags/*:refs/tags/*'
    sign_tag "$config" "$tag" "$skip_soak" "$single_source"
    return
  fi
  drafts="$(gh release list --repo "$repo" --json tagName,isDraft --jq '.[] | select(.isDraft) | .tagName')"
  while IFS= read -r draft; do
    [[ -n "$draft" ]] || continue
    [[ "$draft" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "skipped draft with tag $draft" >&2; status=1; continue; }
    args=(--config "$config")
    [[ "$skip_soak" == 0 ]] || args+=(--skip-soak)
    "$BASH" "${BASH_SOURCE[0]}" "${args[@]}" "$draft" < /dev/null || status=1
  done <<<"$drafts"
  return "$status"
}

[[ "${BASH_SOURCE[0]}" != "$0" ]] || main "$@"

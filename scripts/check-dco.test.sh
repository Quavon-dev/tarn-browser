#!/usr/bin/env bash
set -euo pipefail
script="$(cd "$(dirname "$0")" && pwd)/check-dco.sh"
repo="$(mktemp -d)"
trap 'rm -rf "$repo"' EXIT
cd "$repo"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
git init -q
git config user.name "A Person"
git config user.email "a.person@example.com"
commit() { echo "$1" > "$1"; git add "$1"; git commit -q -m "change $1" "${@:2}"; }
commit base
base="$(git rev-parse HEAD)"
status=0
expect() {
  local want=$1 got=0
  "$script" "$base" HEAD >/dev/null 2>&1 || got=1
  [[ "$got" == "$want" ]] || { echo "check-dco: $2: exit $got, expected $want" >&2; status=1; }
}
commit one -s
expect 0 "signed-off commit"
git checkout -q -b side "$base"
commit side -s
git checkout -q -
git merge -q --no-ff --no-edit side
expect 0 "unsigned merge commit that only combines its parents"
git reset -q --hard HEAD^
git merge -q --no-ff --no-commit side
echo evil > evil; git add evil
git commit -q --no-edit
expect 1 "unsigned merge commit that adds its own change"
git reset -q --hard HEAD^
commit two -m "Signed-off-by: Someone Else <else@example.com>"
expect 1 "sign-off by another email"
git reset -q --hard HEAD^
commit three
expect 1 "commit without sign-off"
git reset -q --hard HEAD^
commit four -m "Signed-off-by: A Person <a.personXexample.com>"
expect 1 "email dot must not match any character"
git reset -q --hard HEAD^
for email in "123+a.person@users.noreply.github.com" "41898282+github-actions[bot]@users.noreply.github.com"; do
  git config user.email "$email"
  commit "five-${email%%@*}" -s
  expect 0 "sign-off by $email"
  git reset -q --hard HEAD^
done
git config user.email "x|.*"
commit six -m "Signed-off-by: Anyone <evil@x>"
expect 1 "email read as a pattern"
"$script" deadbeefdeadbeefdeadbeefdeadbeefdeadbeef HEAD >/dev/null 2>&1 &&
  { echo "check-dco: unknown base: exit 0, expected failure" >&2; status=1; }
exit "$status"

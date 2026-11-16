#!/usr/bin/env bash
# The agreement a person accepts before an AI agent commits for them (AGENTS.md, "AI agreement").
# The acceptance is kept in the clone's common Git directory, so no commit can ever contain it.
# Agents run main's copy (AGENTS.md, "AI agreement"), which works from anywhere in the clone.
#   show     print the agreement for the identity commits are made with
#   check    exit 0 if that identity accepted this version
#   accept   record the acceptance (only after the person said yes in this session)
# Raise VERSION whenever the text changes, so everyone is asked again.
set -euo pipefail

VERSION=1
ident="$(git var GIT_AUTHOR_IDENT)"
name="${ident% <*}"
email="${ident#*<}"
email="${email%%>*}"
file="$(git rev-parse --path-format=absolute --git-common-dir)/tarn-ai-agreement"

text() {
  cat <<EOF
Tarn AI-assisted contribution agreement, version $VERSION

For every contribution I, $name <$email>, make to Tarn with an AI agent:

1. I am the author and answer for all of it; the agent is a tool, never an author.
2. I read and understand every change the agent makes before it is committed, pushed or submitted.
3. I make sure it is correct, that the checks in AGENTS.md pass, that it keeps
   docs/threat-model.md intact, and that it contains no code whose origin and licence I cannot
   name.
4. I have the right to submit it under GPL-3.0-or-later and certify the Developer Certificate of
   Origin 1.1 (https://developercertificate.org) for it.
5. The agent may add my Signed-off-by line to the commits it makes for me; each one is my
   certification under 2-4.
EOF
}

case "${1:-}" in
  show)
    text ;;
  check)
    [[ -n "$email" && -f "$file" ]] && grep -qxF "accepted: v$VERSION $email" "$file" ;;
  accept)
    [[ -n "$name" && -n "$email" ]] || { echo "set git config user.name and user.email first" >&2; exit 1; }
    { text; echo; echo "accepted: v$VERSION $email"; echo "date: $(date -u +%FT%TZ)"; } > "$file"
    echo "Recorded in $file (inside the Git directory, never committed)." ;;
  *)
    echo "usage: $0 show|check|accept" >&2
    exit 2 ;;
esac

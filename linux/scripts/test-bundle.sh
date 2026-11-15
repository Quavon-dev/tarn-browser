#!/bin/sh
# Builds a filter bundle signed with a throwaway key for the filter-list exit check; prints the
# public key in hex for TARN_TEST_FILTER_KEY (debug builds only read it).
set -e
root=$(cd "$(dirname "$0")/../.." && pwd)
exec docker run --rm -v tarn-cargo:/root/.cargo/registry -v tarn-rustup:/root/.rustup \
  -v tarn-linux-p2-root-target:/target -e CARGO_TARGET_DIR=/target -v "$root":/work -w /work tarn-linux-fedora sh -c '
set -e
cargo build -q -p filter-bundle
keys=$(/target/debug/filter-bundle keygen)
seed=$(echo "$keys" | sed -n "s/^FILTER_SIGNING_KEY=//p")
echo "$keys" | sed -n "s/^public key (bytes): //p" | tr -d "[] " | tr "," "\n" | xargs printf "%02x" > /work/linux/target-test-bundle.key
rm -rf /tmp/prepared /work/linux/target-test-bundle && mkdir -p /tmp/prepared
printf "tarn-ad-marker\n" > /tmp/prepared/ads.txt
printf "[{\"name\":\"ads.txt\",\"category\":\"ads\",\"kind\":\"filters\",\"count\":1}]" > /tmp/prepared/entries.json
FILTER_SIGNING_KEY=$seed /target/debug/filter-bundle build --in /tmp/prepared --version $(date +%s) --out /work/linux/target-test-bundle/bundle
'

#!/bin/bash -eu
cd "$SRC/tarn-browser"
cargo fuzz build -O --fuzz-dir fuzz
release=fuzz/target/x86_64-unknown-linux-gnu/release
for source in fuzz/fuzz_targets/*.rs; do
  target="$(basename "$source" .rs)"
  cp "$release/$target" "$OUT/"
  if [[ -d "fuzz/seeds/$target" ]]; then
    (cd "fuzz/seeds/$target" && zip -q -r "$OUT/${target}_seed_corpus.zip" .)
  fi
done

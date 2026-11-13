#!/usr/bin/env bash
# Regenerates tests/fixtures/l7-D7/ (run from that directory; minisign 0.11, `MINISIGN=` picks one).
# Two signed stable releases whose payload pins are the small files under payload/, signed by the
# compiled DEV key (generation 0: ../minisign/test.key = keys/manifest-pub.key). Deterministic:
# fixed payload bytes, a fixed trusted comment and Ed25519 make a re-run byte-identical.
set -euo pipefail
MS=${MINISIGN:-minisign}
T=../minisign/test.key
TC="l7-D7 fixture"
G0=$(tail -n1 ../minisign/test.pub)
RELS="2026.10.12 2026.11.02"
mkdir -p payload
for rel in $RELS; do
  for arch in x86_64 aarch64; do
    for i in $(seq 1 64); do printf 'l7-D7 %s payload %s line %02d\n' "$rel" "$arch" "$i"; done \
      > "payload/$rel-$arch.bin"
  done
done
pin() { # <release> <arch>: one [payload.<triple>] table
  local f="payload/$1-$2.bin"
  printf '[payload.%s-unknown-linux-musl]\nurl = "https://example.invalid/v%s/payload-%s.tar.gz"\n' "$2" "$1" "$2"
  printf 'sha256 = "%s"\nsize = %s\n\n' "$(sha256sum "$f" | cut -d' ' -f1)" "$(stat -c %s "$f")"
}
manifest() { # <release>: the canonical fixture, its dates and payload pins replaced, keychain = generation 0
  sed -e '/^\[\[keychain\]\]/,$d' -e "s/2026\.10\.12/$1/g" ../manifest/channel-111-stable.toml \
    | awk '/^\[payload\./{skip=1; next} /^\[/{skip=0} !skip' > body.tmp
  { pin "$1" x86_64; pin "$1" aarch64; } > pins.tmp
  awk '/^\[apk\.termux\]$/{while ((getline l < "pins.tmp") > 0) print l} {print}' body.tmp
  rm -f body.tmp pins.tmp
  printf '[[keychain]]\ngeneration = 0\npubkey = "%s"\n' "$G0"
}
for rel in $RELS; do
  mkdir -p "$rel"
  manifest "$rel" > "$rel/channel-111-stable.toml"
  "$MS" -S -W -s "$T" -t "$TC" -m "$rel/channel-111-stable.toml" -x "$rel/channel-111-stable.toml.minisig" >/dev/null
done

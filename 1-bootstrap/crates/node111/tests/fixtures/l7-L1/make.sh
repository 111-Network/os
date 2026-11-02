#!/usr/bin/env bash
# Regenerates tests/fixtures/l7-L1/ (run from that directory).
set -euo pipefail
MS=${MINISIGN:-minisign}
T=../minisign/test.key           # generation 0 = the compiled DEV key (keys/manifest-pub.key)
O=../minisign/other.key          # an unrelated key (the wrong previous)
BASE=../manifest/channel-111-stable.toml
TC="l7-L1 fixture"               # fixed trusted comment: Ed25519 is deterministic, so a re-run is byte-identical
[ -f gen1.key ] || "$MS" -G -W -p gen1.pub -s gen1.key
[ -f gen2.key ] || "$MS" -G -W -p gen2.pub -s gen2.key
G0=$(tail -n1 ../minisign/test.pub); G1=$(tail -n1 gen1.pub); G2=$(tail -n1 gen2.pub)
# link <pubkey> <signing key> -> the .minisig text over the pubkey's exact bytes (no newline)
link() { printf '%s' "$1" > link.txt; "$MS" -S -W -s "$2" -t "$TC" -m link.txt -x link.minisig >/dev/null; cat link.minisig; rm -f link.txt link.minisig; }
# head of the canonical fixture, everything before its keychain
body() { sed '/^\[\[keychain\]\]/,$d' "$BASE" | sed "s/^channel = \"stable\"/channel = \"$1\"/"; }
entry() { printf '[[keychain]]\ngeneration = %s\npubkey = "%s"\n' "$1" "$2"; [ -z "${3:-}" ] || printf 'signature_by_previous = """\n%s\n"""\n' "$3"; printf '\n'; }
emit() { mkdir -p "$1"; cat > "$1/channel-111-stable.toml"; "$MS" -S -W -s "$2" -t "$TC" -m "$1/channel-111-stable.toml" -x "$1/channel-111-stable.toml.minisig" >/dev/null; }
L01=$(link "$G1" "$T"); L01_BAD=$(link "$G1" "$O"); L12_BAD=$(link "$G2" "$T")
{ body beta;   entry 0 "$G0"; }                                              | emit beta "$T"
{ body stable; entry 0 "$G0"; entry 1 "$G1" "$L01"; }                        | emit gen1 gen1.key
{ body stable; entry 0 "$G0"; entry 1 "$G1" "$L01"; }                        | emit gen1-root-signed "$T"
{ body stable; entry 0 "$G0"; entry 1 "$G1" "$L01_BAD"; }                    | emit broken-link gen1.key
{ body stable; entry 0 "$G0"; entry 1 "$G1" "$L01"; entry 2 "$G2" "$L12_BAD"; } | emit gen2-broken gen1.key
{ body stable; entry 0 "$G0"; entry 1 "$G1" "$L01"; entry 2 "$G2" "$L12_BAD"; } | emit gen2-broken-gen2-signed gen2.key
{ body stable; entry 0 "RWRoot00000000000000000000000000000000000000000="; }   | emit unlisted-root "$T"

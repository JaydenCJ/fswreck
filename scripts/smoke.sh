#!/usr/bin/env bash
# Smoke test: builds fswreck, generates a full hostile tree into a temp
# directory, verifies it, tampers with it in four distinct ways, checks that
# every tampering is reported, and cleans the tree up through the CLI
# (including the mode-000 directories that defeat plain `rm -rf`).
# Self-contained: temp dirs only, no network, idempotent.
set -euo pipefail

cd "$(dirname "$0")/.."

fail() { echo "SMOKE FAIL: $*" >&2; exit 1; }

echo "[smoke] building..."
cargo build --quiet
BIN=target/debug/fswreck

WORK=$(mktemp -d "${TMPDIR:-/tmp}/fswreck-smoke.XXXXXX")
cleanup() {
    # The tree contains chmod-000 directories; use the tool's own repair
    # logic, then drop the (plain) workdir.
    [ -d "$WORK/wreck" ] && "$BIN" clean --force "$WORK/wreck" >/dev/null 2>&1 || true
    rm -rf "$WORK"
}
trap cleanup EXIT

# --- 1. version/help sanity --------------------------------------------------
"$BIN" --version | grep -q '^fswreck 0\.1\.0$' || fail "--version mismatch"
"$BIN" --help | grep -q 'COMMANDS:' || fail "--help missing sections"
"$BIN" modules | grep -q 'symlinks' || fail "modules listing incomplete"

# --- 2. plan is deterministic and disk-free ----------------------------------
echo "[smoke] fswreck plan"
"$BIN" plan --seed 7 > "$WORK/plan1.txt"
"$BIN" plan --seed 7 > "$WORK/plan2.txt"
cmp -s "$WORK/plan1.txt" "$WORK/plan2.txt" || fail "plan output not deterministic"
grep -q 'names/-rf' "$WORK/plan1.txt" || fail "plan missing the -rf trap"
grep -q 'symlinks/self -> self' "$WORK/plan1.txt" || fail "plan missing the self-loop"

# --- 3. generate the full hostile tree ---------------------------------------
echo "[smoke] fswreck generate (all six modules)"
"$BIN" generate "$WORK/wreck" | tee "$WORK/gen.out"
grep -q 'generated 317 entries' "$WORK/gen.out" || fail "unexpected entry count"
[ -p "$WORK/wreck/exotic/fifo.pipe" ] || fail "FIFO not created"
[ -L "$WORK/wreck/symlinks/self" ] || fail "self-loop symlink not created"
[ -f "$WORK/wreck/names/-rf" ] || fail "-rf file not created"
[ "$(stat -c %a "$WORK/wreck/perms/no-access-dir")" = "0" ] \
    || fail "no-access-dir is not mode 000"
SPARSE_BYTES=$(stat -c %s "$WORK/wreck/exotic/sparse.bin")
[ "$SPARSE_BYTES" = "1048576" ] || fail "sparse file logical size $SPARSE_BYTES"

# Generating into the same (now non-empty) directory must be refused.
if "$BIN" generate "$WORK/wreck" 2> "$WORK/refuse.err"; then
    fail "generate clobbered a non-empty directory"
fi
grep -q 'not empty' "$WORK/refuse.err" || fail "refusal lacks a reason"

# --- 4. verify: clean pass, then four detected tamperings --------------------
echo "[smoke] fswreck verify (pristine)"
"$BIN" verify "$WORK/wreck" | grep -q 'verified 317 entries.*: OK' \
    || fail "pristine tree did not verify"

echo "[smoke] tampering with the tree"
rm "$WORK/wreck/exotic/empty.txt"                          # deletion
chmod 666 "$WORK/wreck/perms/read-only.txt"                # mode drift
printf 'x' >> "$WORK/wreck/symlinks/payload.txt"           # content change
touch "$WORK/wreck/names/impostor"                         # unexpected entry

if "$BIN" verify "$WORK/wreck" > "$WORK/verify.out"; then
    fail "verify exited 0 on a tampered tree"
fi
grep -q 'exotic/empty.txt: missing' "$WORK/verify.out" || fail "deletion not reported"
grep -q 'read-only.txt: mode 666, manifest says 444' "$WORK/verify.out" \
    || fail "mode drift not reported"
grep -q 'payload.txt: size 65 bytes, manifest says 64' "$WORK/verify.out" \
    || fail "content change not reported"
grep -q 'names/impostor: unexpected entry' "$WORK/verify.out" \
    || fail "extra file not reported"
grep -q '4 problems$' "$WORK/verify.out" || fail "problem count wrong"
# Verification must restore the modes it relaxed to look inside.
[ "$(stat -c %a "$WORK/wreck/perms/no-access-dir")" = "0" ] \
    || fail "verify did not restore restrictive modes"

# --- 5. determinism across generations ---------------------------------------
echo "[smoke] seed reproducibility"
"$BIN" generate "$WORK/again" --seed 42 >/dev/null
cmp -s "$WORK/wreck/.fswreck-manifest.json" "$WORK/again/.fswreck-manifest.json" \
    || fail "same seed produced different manifests"
"$BIN" clean "$WORK/again" >/dev/null

# --- 6. clean repairs permissions and deletes everything ---------------------
echo "[smoke] fswreck clean"
"$BIN" clean "$WORK/wreck" | grep -q 'removed' || fail "clean did not report removal"
[ ! -e "$WORK/wreck" ] || fail "tree still present after clean"

echo "SMOKE OK"

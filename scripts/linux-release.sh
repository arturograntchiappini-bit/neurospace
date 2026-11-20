#!/bin/sh
# Build Neurospace for Linux and pack it. Run from the repository root inside
# rust:1-bullseye (Debian 11, glibc 2.31); needs no apt package.
set -eu

MAX_GLIBC=2.31
target_dir="${CARGO_TARGET_DIR:-target}"
version=$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' Cargo.toml)
[ -n "$version" ] || { echo "cannot read the workspace version from Cargo.toml" >&2; exit 1; }
name="neurospace-$version-linux-x86_64"

cargo build --release --locked -p ns-cli -p ns-app

# Prints the highest GLIBC_x.y version a binary needs; fails if it exceeds MAX_GLIBC.
highest_glibc() {
    highest=$(objdump -T "$1" | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/^GLIBC_//' | sort -Vu | tail -n 1)
    newest=$(printf '%s\n%s\n' "$highest" "$MAX_GLIBC" | sort -V | tail -n 1)
    if [ "$newest" != "$MAX_GLIBC" ]; then
        echo "$1 needs GLIBC_$highest, newer than GLIBC_$MAX_GLIBC" >&2
        exit 1
    fi
    echo "$highest"
}

for bin in ns neurospace; do
    echo "$bin: highest glibc $(highest_glibc "$target_dir/release/$bin")"
done

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/$name"
cp "$target_dir/release/neurospace" "$target_dir/release/ns" "$stage/$name/"
cp packaging/linux/neurospace.desktop README.md "$stage/$name/"
cp assets/logo/neurospace-256.png "$stage/$name/neurospace.png"
cp crates/ns-app/assets/Inter-LICENSE.txt "$stage/$name/"

mkdir -p dist
rm -f "dist/$name.tar.gz"
tar -czf "dist/$name.tar.gz" -C "$stage" "$name"
echo "wrote dist/$name.tar.gz"

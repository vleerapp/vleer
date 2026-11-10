#!/usr/bin/env bash
set -euo pipefail

pkg="${1:?usage: publish-aur.sh <package>}"
: "${PKGVER:?}"
root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"

if [ -n "${AUR_SSH_KEY:-}" ]; then
  install -d -m 700 ~/.ssh
  printf '%s\n' "$AUR_SSH_KEY" > ~/.ssh/aur
  chmod 600 ~/.ssh/aur
  ssh-keyscan aur.archlinux.org >> ~/.ssh/known_hosts 2>/dev/null
  export GIT_SSH_COMMAND="ssh -i ~/.ssh/aur -o IdentitiesOnly=yes"
  git clone "ssh://aur@aur.archlinux.org/${pkg}.git" "$work/repo"
else
  echo "AUR_SSH_KEY not set, dry run"
  git init -q "$work/repo"
fi

sed \
  -e "s|@PKGVER@|${PKGVER}|g" \
  -e "s|@SHA_X86_64@|${SHA_X86_64:-}|g" \
  -e "s|@SHA_AARCH64@|${SHA_AARCH64:-}|g" \
  -e "s|@SHA_SOURCE@|${SHA_SOURCE:-}|g" \
  "$root/packaging/aur/${pkg}/PKGBUILD.in" > "$work/repo/PKGBUILD"

docker run --rm --network none -v "$work/repo:/pkg" archlinux:base bash -c \
  'useradd -m b && install -o b -m 644 /pkg/PKGBUILD /home/b/PKGBUILD && su b -c "cd ~ && makepkg --printsrcinfo" > /pkg/.SRCINFO'

cd "$work/repo"
git config user.name "PandaDEV"
git config user.email "pandadev.net@proton.me"
git add PKGBUILD .SRCINFO
if git diff --cached --quiet; then
  echo "no changes"
  exit 0
fi

if [ -z "${AUR_SSH_KEY:-}" ]; then
  cat PKGBUILD .SRCINFO
  exit 0
fi

git commit -m "${COMMIT_MESSAGE:-update to ${PKGVER}}"
git push origin HEAD:master

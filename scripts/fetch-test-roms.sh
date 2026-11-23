#!/usr/bin/env bash
# Downloads the community test ROM bundle (Blargg, Mooneye, dmg-acid2, ...)
# from c-sp/game-boy-test-roms into ./test-roms. The ROMs are not committed;
# integration tests skip themselves when this folder is missing.
set -euo pipefail

VERSION="${TEST_ROMS_VERSION:-v7.0}"
URL="https://github.com/c-sp/game-boy-test-roms/releases/download/${VERSION}/game-boy-test-roms-${VERSION}.zip"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${ROOT}/test-roms"

if [[ -f "${DEST}/.version" && "$(cat "${DEST}/.version")" == "${VERSION}" ]]; then
  echo "test ROMs ${VERSION} already present in ${DEST}"
  exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

echo "downloading ${URL}"
curl -fsSL --retry 3 -o "${tmp}/roms.zip" "${URL}"
rm -rf "${DEST}"
mkdir -p "${DEST}"
unzip -q "${tmp}/roms.zip" -d "${DEST}"
echo "${VERSION}" > "${DEST}/.version"
echo "extracted to ${DEST}"

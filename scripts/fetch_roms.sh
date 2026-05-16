#!/usr/bin/env bash
# Downloads BBC Micro Model B ROMs (MOS 1.20 + BASIC II) from mdfs.net, which has
# hosted a public archive of Acorn ROMs for decades. These ROMs are © Acorn /
# successor rightholders; download for emulation use only.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="${SCRIPT_DIR}/../roms"
mkdir -p "${DEST}"

declare -A FILES=(
    ["os120.rom"]="https://mdfs.net/System/ROMs/AcornMOS/BBC_120/MOS120"
    ["basic2.rom"]="https://mdfs.net/System/ROMs/AcornMOS/BBC_120/BASIC200"
    ["dfs098.rom"]="https://mdfs.net/System/ROMs/Filing/Disk/Acorn/DFS098"
)

for fname in "${!FILES[@]}"; do
    url="${FILES[$fname]}"
    out="${DEST}/${fname}"
    if [[ -f "${out}" ]]; then
        echo "skip ${fname} (already present at ${out})"
        continue
    fi
    echo "fetching ${fname} from ${url}"
    if ! curl -fL "${url}" -o "${out}"; then
        echo "  failed; you may need to provide ${fname} manually."
        rm -f "${out}"
        continue
    fi
    actual_size=$(stat -c%s "${out}")
    case "${actual_size}" in
        16384|8192|4096) ;;
        *) echo "  warning: ${fname} is ${actual_size} bytes (expected 16384 / 8192 / 4096). Verify the source." ;;
    esac
done

echo "done; ROMs are in ${DEST}"

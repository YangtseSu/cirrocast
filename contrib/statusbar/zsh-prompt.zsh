# SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Zsh prompt snippet that shows the one-line cirrocast status probe. Source it
# from ~/.zshrc; a `precmd` hook renders the probe into a cache file at most
# once every CIRROCAST_STATUS_INTERVAL seconds (default 900) and PROMPT reads
# that file back, so an interactive prompt never blocks on the network.
#
# cirrocast-example: cirrocast status -q --format '%c %t'

CIRROCAST_STATUS_INTERVAL=${CIRROCAST_STATUS_INTERVAL:-900}

_cirrocast_status_render() {
    cirrocast status -q --format '%c %t'
}

_cirrocast_status_file() {
    printf '%s' "${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/cirrocast/status"
}

# Render only when the cached line is missing or older than the interval; never
# write anything to the terminal, whatever the probe or the filesystem does.
_cirrocast_status_update() {
    local file now mtime
    file=$(_cirrocast_status_file)
    if [[ -f "$file" ]]; then
        now=$(date +%s 2>/dev/null) || return 0
        mtime=$(date -r "$file" +%s 2>/dev/null) || mtime=0
        if (( now - mtime < CIRROCAST_STATUS_INTERVAL )); then
            return 0
        fi
    fi
    mkdir -p "${file:h}" 2>/dev/null || return 0
    _cirrocast_status_render >"$file" 2>/dev/null || return 0
}

_cirrocast_status_show() {
    cat "$(_cirrocast_status_file)" 2>/dev/null
}

# precmd hook: refresh the cache before each prompt, then inline the cached line.
precmd_functions+=(_cirrocast_status_update)
PROMPT='$(_cirrocast_status_show) '"$PROMPT"

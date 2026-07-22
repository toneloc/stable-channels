#!/bin/sh
# nginx docker-entrypoint.d hook: once sc-lsp has generated its API key
# (<storage_dir>/<network>/api_key, raw bytes), publish it hex-encoded at
# /setup so the user can copy it into the GUI connection screen without SSH.
# The sc-lsp storage dir is mounted read-only at /run/sc-lsp; behind Umbrel's
# app_proxy the page is gated by the user's Umbrel login.
find_api_key() {
    preferred="${SC_NETWORK:-}"
    if [ -z "$preferred" ] && [ -s /run/sc-lsp/network ]; then
        preferred="$(sed -n '1p' /run/sc-lsp/network 2>/dev/null || true)"
    fi
    if [ -n "$preferred" ] && [ -s "/run/sc-lsp/${preferred}/api_key" ]; then
        printf '%s\n' "/run/sc-lsp/${preferred}/api_key"
        return 0
    fi
    if [ -s /run/sc-lsp/bitcoin/api_key ]; then
        printf '%s\n' /run/sc-lsp/bitcoin/api_key
        return 0
    fi
    for candidate in /run/sc-lsp/*/api_key; do
        [ -s "$candidate" ] || continue
        printf '%s\n' "$candidate"
        return 0
    done
    return 1
}

(
    until key_file="$(find_api_key)"; do sleep 5; done
    hex="$(od -An -tx1 "$key_file" | tr -d ' \n')"
    mkdir -p /usr/share/nginx/html/setup
    # Bare key for the GUI's zero-input auto-connect (fetched same-origin).
    printf '%s\n' "$hex" > /usr/share/nginx/html/setup/key.txt
    cat > /usr/share/nginx/html/setup/index.html <<EOF
<!doctype html><meta charset="utf-8"><title>Stable Channels LSP — Setup</title>
<body style="font-family:system-ui;max-width:40em;margin:4em auto">
<h2>GUI API key</h2>
<p>Paste this into the GUI's connection screen (the Server URL field is ignored):</p>
<pre style="background:#eee;padding:1em;word-break:break-all;white-space:pre-wrap">$hex</pre>
<p><a href="/">Open the GUI</a></p>
</body>
EOF
) &

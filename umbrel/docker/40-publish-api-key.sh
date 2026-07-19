#!/bin/sh
# nginx docker-entrypoint.d hook: once sc-lsp has generated its API key
# (<storage_dir>/<network>/api_key, raw bytes), publish it hex-encoded at
# /setup so the user can copy it into the GUI connection screen without SSH.
# The sc-lsp storage dir is mounted read-only at /run/sc-lsp; behind Umbrel's
# app_proxy the page is gated by the user's Umbrel login.
(
    key_file="/run/sc-lsp/${SC_NETWORK:-bitcoin}/api_key"
    until [ -s "$key_file" ]; do sleep 5; done
    hex="$(od -An -tx1 "$key_file" | tr -d ' \n')"
    mkdir -p /usr/share/nginx/html/setup
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

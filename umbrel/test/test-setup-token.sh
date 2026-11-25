#!/bin/sh
# Check the GUI nginx serves the current gui.macaroon at /setup/token.txt and nowhere else (stock nginx:alpine, no wasm build).
set -eu

HERE="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
DOCKER_DIR="$HERE/../docker"
IMAGE="${SC_NGINX_IMAGE:-nginx:alpine}"
TMP="$(mktemp -d)"
# sc-lsp's storage dir, mounted read-only at /run/sc-lsp as in the Umbrel compose.
DATA="$TMP/sc-lsp"
NAME="sc-token-test-$$"
PORT=$(( 20000 + $(od -An -N2 -tu2 /dev/urandom | tr -d ' ') % 20000 ))
HTTPS_PORT=$((PORT + 1))
FAILED=0
TOKEN=""

cleanup() {
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    # The daemon's files belong to uid 1000 and are private, so a root container removes them.
    docker run --rm -v "$TMP:/t" "$IMAGE" rm -rf /t/sc-lsp >/dev/null 2>&1 || true
    rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

check() {
    if [ "$2" = "$3" ]; then echo "ok   - $1"; else echo "FAIL - $1 (want '$3', got '$2')"; FAILED=1; fi
}

code() { curl -sk --max-time 5 -o /dev/null -w '%{http_code}' "$@" || true; }

# Runs a command as root on the storage dir, as Umbrel's pre-start hook does.
as_root() { docker run --rm -v "$DATA:/d" "$IMAGE" sh -c "$1"; }

# Runs a command as sc-lsp does: uid 1000 with private files.
as_daemon() { docker run --rm --user 1000:1000 -v "$DATA:/d" "$IMAGE" sh -c "umask 077; $1"; }

# Replaces gui.macaroon the way sc-lsp does (hex and a newline, written aside, then renamed).
write_token() {
    TOKEN="$1"
    as_daemon "printf '%s\n' '$1' > /d/signet/.gui.tmp && mv -f /d/signet/.gui.tmp /d/signet/gui.macaroon"
}

empty_token() { as_daemon ": > /d/signet/.gui.tmp && mv -f /d/signet/.gui.tmp /d/signet/gui.macaroon"; }

remove_token() { as_daemon "rm -f /d/signet/gui.macaroon"; }

# usage: wait_body <label> <url> <want>; the served copy follows the file within a few seconds.
wait_body() {
    i=0
    got="$(curl -sk --max-time 5 "$2" || true)"
    while [ "$got" != "$3" ] && [ "$i" -lt 40 ]; do
        sleep 0.5
        i=$((i + 1))
        got="$(curl -sk --max-time 5 "$2" || true)"
    done
    check "$1" "$got" "$3"
}

# usage: wait_code <label> <url> <want>
wait_code() {
    i=0
    got="$(code "$2")"
    while [ "$got" != "$3" ] && [ "$i" -lt 40 ]; do
        sleep 0.5
        i=$((i + 1))
        got="$(code "$2")"
    done
    check "$1" "$got" "$3"
}

# usage: only_exact <label> <base-url> <raw-target>; the target is sent byte for byte.
only_exact() {
    body="$TMP/body"
    : > "$body"
    status="$(curl -sk --max-time 5 --path-as-is -o "$body" -w '%{http_code}' "$2$3" || true)"
    check "$1" "$status" 404
    check "$1: no token" "$(grep -c "$TOKEN" "$body" || true)" 0
}

# Realistic ownership: the storage dir belongs to uid 1000; the network dir and the token are private to it.
mkdir "$DATA"
as_root "chown 1000:1000 /d && chmod 0755 /d && printf 'signet\n' > /d/network"
as_daemon "mkdir -m 0700 /d/signet"

# Stock nginx has no openssl CLI; the real image installs it at build time.
printf '#!/bin/sh\napk add --no-cache openssl >/dev/null 2>&1\n' > "$TMP/25-openssl.sh"
chmod 0755 "$TMP/25-openssl.sh"

# usage: start_gui <allow-cidrs> [extra docker args...]
start_gui() {
    cidrs="$1"; shift
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker run -d --name "$NAME" -p "127.0.0.1:$PORT:80" -p "127.0.0.1:$HTTPS_PORT:8443" \
        -e "SC_SETUP_KEY_ALLOW_CIDRS=$cidrs" -e SC_SETUP_KEY_ALLOW_HOSTS=none "$@" \
        -v "$DOCKER_DIR/lsp-gui-nginx.conf.template:/etc/nginx/templates/default.conf.template:ro" \
        -v "$DOCKER_DIR/gui-locations.inc.template:/etc/nginx/templates/gui-locations.inc.template:ro" \
        -v "${OPENSSL_HOOK:-$TMP/25-openssl.sh}:/docker-entrypoint.d/25-openssl.sh:ro" \
        -v "${TLS_SCRIPT:-$DOCKER_DIR/30-gui-tls.sh}:/docker-entrypoint.d/30-gui-tls.sh:ro" \
        -v "$DOCKER_DIR/40-provision-gui-token.sh:/docker-entrypoint.d/40-provision-gui-token.sh:ro" \
        -v "$DATA:/run/sc-lsp:ro" \
        "$IMAGE" >/dev/null
}

wait_up() { # url
    i=0
    until curl -sk -o /dev/null "$1" 2>/dev/null; do
        i=$((i + 1))
        [ "$i" -lt 80 ] || { echo "FAIL - nginx did not answer $1"; docker logs "$NAME" 2>&1 | tail -8; exit 1; }
        sleep 0.5
    done
}

# Plain server with the allowlist open to everyone, for the content checks.
start_gui 0.0.0.0/0
BASE="http://127.0.0.1:$PORT"
URL="$BASE/setup/token.txt"
wait_up "$BASE/"
check "503 while gui.macaroon is missing" "$(code "$URL")" 503
write_token aabbcc
wait_body "serves the token sc-lsp keeps private" "$URL" aabbcc
check "no-store header" "$(curl -sI "$URL" | tr -d '\r' | grep -ci '^cache-control: no-store')" 1
write_token ddeeff
wait_body "serves the replaced token without restart" "$URL" ddeeff
check "setup page answers" "$(code "$BASE/setup/")" 200
check "setup page carries no token" "$(curl -s "$BASE/setup/" | grep -c ddeeff || true)" 0
check "setup redirect answers" "$(code "$BASE/setup")" 302
remove_token
wait_code "503 again once gui.macaroon is gone" "$URL" 503
empty_token
sleep 4
check "an empty gui.macaroon is not served" "$(code "$URL")" 503
write_token 0201abcdef
wait_body "serves the token again" "$URL" 0201abcdef

n=0
for target in /%73etup/token.txt /setup/%74oken.txt /setup/token%2etxt /%2Fsetup/token.txt //setup/token.txt \
    /setup//token.txt /api/../setup/token.txt /./setup/token.txt /setup/./token.txt /setup/x/../token.txt \
    /%73etup/ /%73etup/index.html /setup/./ /%73etup; do
    n=$((n + 1))
    only_exact "setup files answer only on their exact paths (case $n)" "$BASE" "$target"
done

n=0
for target in /api/api/register-email /api/api/register-push /api/api/channel-exists \
    /api/api/funding-outpoints /api/%61pi/register-email /api//api/register-email; do
    n=$((n + 1))
    check "only GUI routes reach the daemon (case $n)" "$(code --path-as-is -X POST "$BASE$target")" 404
done

# A caller outside the allowlist must be refused; the GUI root stays open.
start_gui 203.0.113.9
wait_up "$BASE/"
check "non-allowlisted caller refused" "$(code "$URL")" 403
check "GUI root still served" "$(code "$BASE/")" 200

# HTTPS listener with a generated self-signed pair.
remove_token
start_gui 0.0.0.0/0 -e SC_GUI_TLS=self-signed
SBASE="https://127.0.0.1:$HTTPS_PORT"
SURL="$SBASE/setup/token.txt"
wait_up "$SBASE/"
check "https: 503 while missing" "$(code "$SURL")" 503
check "https: GUI root served" "$(code "$SBASE/")" 200
write_token aabbcc
wait_body "https: serves the token" "$SURL" aabbcc
write_token ddeeff
wait_body "https: serves replaced token" "$SURL" ddeeff
only_exact "https: setup files answer only on their exact paths" "$SBASE" /%73etup/token.txt
check "https: only GUI routes reach the daemon" "$(code --path-as-is -X POST "$SBASE/api/api/register-email")" 404

# HTTPS listener with a mounted certificate and key.
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=test" -keyout "$TMP/pair.key" -out "$TMP/pair.crt" >/dev/null 2>&1
chmod 0644 "$TMP/pair.key"
start_gui 0.0.0.0/0 -e SC_GUI_TLS_CERT=/tls/pair.crt -e SC_GUI_TLS_KEY=/tls/pair.key -v "$TMP/pair.crt:/tls/pair.crt:ro" -v "$TMP/pair.key:/tls/pair.key:ro"
wait_up "$SBASE/"
check "https with a mounted pair: GUI root served" "$(code "$SBASE/")" 200

# usage: refuses <label> <variable named in the log> [docker env args...]; the container must exit non-zero.
refuses() {
    label="$1"; var="$2"; shift 2
    start_gui 0.0.0.0/0 "$@"
    status="$(timeout 30 docker wait "$NAME" || echo timeout)"
    if [ "$status" = timeout ] || [ "$status" = 0 ]; then status="still running or exit 0"; else status="non-zero"; fi
    check "$label: the container does not start" "$status" non-zero
    check "$label: the log names $var" "$(docker logs "$NAME" 2>&1 | grep -c "30-gui-tls.sh: .*$var" || true)" 1
}

# A set but unusable certificate or key stops the container instead of leaving HTTPS off.
refuses "cert set to a missing file" SC_GUI_TLS_CERT -e SC_GUI_TLS_CERT=/nonexistent/gui.crt -e SC_GUI_TLS_KEY=/nonexistent/gui.key
refuses "only the cert set" SC_GUI_TLS_KEY -e SC_GUI_TLS_CERT=/etc/hostname
refuses "only the key set" SC_GUI_TLS_CERT -e SC_GUI_TLS_KEY=/etc/hostname

# An unknown SC_GUI_TLS value or a self-signed setup that cannot run stops the container too.
refuses "SC_GUI_TLS typo" selfsigned -e SC_GUI_TLS=selfsigned
refuses "SC_GUI_TLS=true" "true" -e SC_GUI_TLS=true
printf '#!/bin/sh\nexit 0\n' > "$TMP/25-noop.sh"
chmod 0755 "$TMP/25-noop.sh"
OPENSSL_HOOK="$TMP/25-noop.sh" refuses "self-signed without openssl" openssl -e SC_GUI_TLS=self-signed
mkdir "$TMP/ro-tls"
refuses "self-signed with an unwritable cert dir" "could not" -e SC_GUI_TLS=self-signed -v "$TMP/ro-tls:/etc/nginx/tls:ro"

if [ "$FAILED" = 0 ]; then echo "PASS"; else echo "FAILED"; exit 1; fi

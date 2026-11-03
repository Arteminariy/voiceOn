#!/bin/sh
# Установка или обновление voiceon-server на Linux-VPS с systemd.
# Запуск от root:
#   curl -fsSL https://raw.githubusercontent.com/Arteminariy/voiceOn/main/deploy/install.sh | sh
# Переменные: VOICEON_VERSION (тег релиза, по умолчанию latest), VOICEON_PORT (47800).
set -eu

REPO="Arteminariy/voiceOn"
VERSION="${VOICEON_VERSION:-latest}"
PORT="${VOICEON_PORT:-47800}"
BIN=/usr/local/bin/voiceon-server
CONF_DIR=/etc/voiceon
CONF="$CONF_DIR/server.toml"
UNIT=/etc/systemd/system/voiceon-server.service

[ "$(id -u)" -eq 0 ] || { echo "нужен root"; exit 1; }
[ "$(uname -m)" = "x86_64" ] || { echo "собрано только под x86_64"; exit 1; }

if [ "$VERSION" = latest ]; then
    BASE="https://github.com/$REPO/releases/latest/download"
else
    BASE="https://github.com/$REPO/releases/download/$VERSION"
fi
RAW="https://raw.githubusercontent.com/$REPO/main/deploy"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
echo "==> скачиваю релиз ($VERSION)"
curl -fsSL "$BASE/voiceon-server-linux-x86_64" -o "$TMP/voiceon-server"
curl -fsSL "$BASE/SHA256SUMS" -o "$TMP/SHA256SUMS"
curl -fsSL "$RAW/voiceon-server.service" -o "$TMP/voiceon-server.service"

echo "==> проверяю sha256"
EXPECTED=$(grep ' voiceon-server-linux-x86_64$' "$TMP/SHA256SUMS" | cut -d' ' -f1)
ACTUAL=$(sha256sum "$TMP/voiceon-server" | cut -d' ' -f1)
[ -n "$EXPECTED" ] && [ "$EXPECTED" = "$ACTUAL" ] || { echo "sha256 не совпал"; exit 1; }

install -m 755 "$TMP/voiceon-server" "$BIN"
install -m 644 "$TMP/voiceon-server.service" "$UNIT"

if [ ! -f "$CONF" ]; then
    if ss -Huln "sport = :$PORT" | grep -q .; then
        echo "UDP-порт $PORT занят, запусти с VOICEON_PORT=другой"; exit 1
    fi
    echo "==> первая установка: придумай парольную фразу комнаты (её же вводят в клиенте)"
    PASS=""
    while [ -z "$PASS" ]; do
        printf "фраза: " > /dev/tty
        stty -echo < /dev/tty
        IFS= read -r PASS < /dev/tty || true
        stty echo < /dev/tty
        echo > /dev/tty
    done
    install -d -m 700 "$CONF_DIR"
    umask 077
    # Экранируем \ и " для TOML-строки.
    ESC=$(printf '%s' "$PASS" | sed 's/\\/\\\\/g; s/"/\\"/g')
    printf 'bind = "0.0.0.0:%s"\npassphrase = "%s"\n' "$PORT" "$ESC" > "$CONF"
    chmod 600 "$CONF"
else
    echo "==> конфиг $CONF уже есть, оставляю как есть"
    PORT=$(sed -n 's/^bind *= *".*:\([0-9]*\)".*/\1/p' "$CONF")
fi

if command -v ufw >/dev/null 2>&1 && ufw status | grep -q "Status: active"; then
    echo "==> открываю UDP $PORT в ufw"
    ufw allow "$PORT/udp" >/dev/null
fi

systemctl daemon-reload
systemctl enable voiceon-server >/dev/null 2>&1
systemctl restart voiceon-server
sleep 1
systemctl --no-pager --lines=5 status voiceon-server || true
echo
echo "==> готово: сервер слушает UDP $PORT"
echo "    логи: journalctl -u voiceon-server -f"

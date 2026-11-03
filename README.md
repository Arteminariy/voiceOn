# voiceOn

Лёгкие голосовые созвоны для своей компании: один маленький relay-сервер на VPS
и клиент с окном на Windows и Linux.

- Голос: Opus 48 кГц моно, 20 мс, 32 кбит/с, FEC; шумодав RNNoise и голосовая активация.
- Сеть: голый UDP, всё зашифровано ChaCha20-Poly1305 ключом из общей парольной фразы.
  Без правильной фразы сервер молча игнорирует пакеты.
- Сервер ничего не декодирует: проверяет пакет и пересылает остальным.
- Смена сети на лету (включил VPN, переключил Wi-Fi) — звонок продолжается,
  сервер узнаёт новый адрес по первому же пакету.

## Загрузки

Готовые сборки — на странице [последнего релиза](https://github.com/Arteminariy/voiceOn/releases/latest):

| Файл | Для чего |
|---|---|
| `voiceon-windows-x86_64.exe` | клиент для Windows 10/11, установка не нужна |
| `voiceon-linux-x86_64` | клиент для Linux (нужна свежая glibc: Arch/CachyOS/Fedora; на старых дистрибутивах собери из исходников) |
| `voiceon-server-linux-x86_64` | сервер, статический бинарник |
| `SHA256SUMS` | контрольные суммы |

Windows при первом запуске может показать SmartScreen («Неизвестный издатель»):
«Подробнее» → «Выполнить в любом случае». На Linux: `chmod +x voiceon-linux-x86_64`.

## Как пользоваться

1. Запусти клиент.
2. Впиши сервер (`host:порт`), ник и пароль комнаты — их даёт тот, кто держит сервер.
3. «Подключиться». Слайдеры — громкость каждого участника, «Настройки» — устройства
   и чувствительность голосовой активации (сдвинь влево, если обрезает начало фраз,
   вправо — если в эфир уходит клавиатура).

Настройки хранятся в `~/.config/voiceon/config.toml` (Linux) или
`%APPDATA%\voiceon\config.toml` (Windows).

### Если играешь на колонках, а не в наушниках

voiceOn не подавляет эхо, и без наушников остальные будут слышать себя.
Включи системный эхоподавитель:

- **Linux (PipeWire):** добавь модуль `echo-cancel` в
  `~/.config/pipewire/pipewire.conf.d/echo-cancel.conf`:
  ```
  context.modules = [
    { name = libpipewire-module-echo-cancel
      args = { library.name = aec/libspa-aec-webrtc } }
  ]
  ```
  перезапусти PipeWire (`systemctl --user restart pipewire`) и выбери в voiceOn
  устройства «Echo-Cancel Source/Sink».
- **Windows:** Параметры → Система → Звук → микрофон → «Улучшения звука» /
  «Эффекты звука»; у многих драйверов (Realtek и др.) есть «Подавление эха».

## Сборка

Нужны Rust и `cmake` (для libopus).

```sh
cargo test --workspace
cargo build --release -p voiceon                                    # клиент Linux
cargo build --release -p voiceon --target x86_64-pc-windows-gnu     # клиент Windows (mingw-w64)
cargo build --release -p voiceon-server --target x86_64-unknown-linux-musl  # сервер, статический
```

## Сервер

Установка и обновление на VPS с systemd (от root), спросит парольную фразу при первой установке:

```sh
curl -fsSL https://raw.githubusercontent.com/Arteminariy/voiceOn/main/deploy/install.sh | sh
```

Порт по умолчанию — UDP 47800 (`VOICEON_PORT=… sh` чтобы сменить); если включён `ufw`,
скрипт откроет порт сам, у хостера с внешним фаерволом открой его в панели.

Вручную:

```sh
install -m 755 voiceon-server /usr/local/bin/
install -d -m 700 /etc/voiceon
install -m 600 deploy/server.toml.example /etc/voiceon/server.toml   # и впиши фразу
install -m 644 deploy/voiceon-server.service /etc/systemd/system/
systemctl daemon-reload && systemctl enable --now voiceon-server
journalctl -u voiceon-server -f
```

Открой в фаерволе UDP-порт из конфига (по умолчанию 47800).

## Проверка без микрофона

Бот подключается, шлёт тон (секунду звучит, секунду молчит) и раз в секунду
печатает пинг, потери, участников и сколько кадров он слышал:

```sh
voiceon --bot --server host:47800 --pass "фраза" --nick bot1 --seconds 20
```

## Устройство

| Крейт | Что внутри |
|---|---|
| `proto` | заголовок пакета, PSK (Argon2id) + ChaCha20-Poly1305, replay-окно, roster/voice |
| `server` | relay: сессии по `session_id`, роуминг адреса, таймауты, периодический roster |
| `client` | cpal → RNNoise/VAD → Opus → UDP; jitter buffer → микшер → cpal; окно egui |

Пакет: `version | kind | room_id | session_id | seq | ciphertext | tag`.
Заголовок открыт, но аутентифицирован; nonce = `session_id ‖ seq`, и каждая сторона
при каждом подключении берёт новый случайный `session_id`, так что nonce не повторяется.

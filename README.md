# voiceOn

Лёгкие голосовые созвоны для своей компании: один маленький relay-сервер на VPS
и клиенты для Windows, Linux и Android.

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
| `voiceon-android.apk` | клиент для Android 8.0 и новее |
| `voiceon-server-linux-x86_64` | сервер, статический бинарник |
| `SHA256SUMS` | контрольные суммы |

Windows при первом запуске может показать SmartScreen («Неизвестный издатель»):
«Подробнее» → «Выполнить в любом случае». На Linux: `chmod +x voiceon-linux-x86_64`.

## Android

**Установка.** Скачай `voiceon-android.apk` со страницы релиза прямо на телефон и открой.
Android спросит, можно ли устанавливать приложения из этого источника (браузера или
файлового менеджера): «Настройки» → включи «Разрешить из этого источника» → назад →
«Установить». С компьютера можно поставить по USB: `adb install voiceon-android.apk`.

**Звонок.** Сервер, ник и пароль — те же, что на компьютере. При первом подключении
приложение попросит доступ к микрофону, уведомлениям и (на Android 12+) к устройствам
Bluetooth — без последнего не заработает Bluetooth-гарнитура. Звонок идёт в фоне и с
выключенным экраном; в уведомлении — число участников и кнопки «Микрофон вкл/выкл» и
«Отключиться». Подключённая гарнитура (проводная или Bluetooth) берётся сама, иначе звук
идёт в разговорный динамик; «Громкая связь» внизу экрана переключает на основной.
Переход с Wi-Fi на мобильную сеть звонок не рвёт. Эхо гасит сам телефон.

**Обновления.** При запуске приложение смотрит последний релиз на GitHub; если там есть
APK новее, на экране входа появится «Доступна vX — Обновить». Приложение скачает его,
проверит подпись тем же ключом, что и у десктопа, и откроет системный установщик. В первый
раз Android попросит разрешить установку из voiceOn — включи и нажми «Обновить» ещё раз.

## Как пользоваться

При первом запуске клиент сам ставит себя в профиль (`%LOCALAPPDATA%\Programs\voiceOn`
или `~/.local/share/voiceon`) и добавляет ярлык в меню «Пуск» / меню приложений —
дальше запускай оттуда. Обновления приходят сами. Крестик прячет окно в трей,
выход — через меню значка в трее.

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

APK собирает Gradle, `.so` ядра — `cargo-ndk` из Gradle-задачи. Нужны JDK 17+, Android SDK
с NDK 30 и `cargo-ndk` (`cargo install cargo-ndk`, плюс
`rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android`).
В `android/local.properties` укажи `sdk.dir=…` (и `cargo.bin=…`, если `cargo-ndk` не в `PATH`):

```sh
cd android
./gradlew :app:assembleDebug     # app/build/outputs/apk/debug/app-debug.apk
./gradlew :app:assembleRelease   # подпись — файл свойств из VOICEON_ANDROID_SIGNING
                                 # или ~/.config/voiceon-android.properties
adb install -r app/build/outputs/apk/release/app-release.apk
```

## Сервер

Установка и обновление на VPS с systemd (от root), спросит парольную фразу при первой установке:

```sh
curl -fsSL https://raw.githubusercontent.com/Arteminariy/voiceOn/main/deploy/install.sh | sh
```

Порт по умолчанию — UDP 47800 (`VOICEON_PORT=… sh` чтобы сменить); если включён `ufw`,
скрипт откроет порт сам, у хостера с внешним фаерволом открой его в панели.

Если по пути (например, на выходе из РФ) режется «неизвестный» UDP, проверь, какие порты
доходят (`tcpdump -ni any udp and src host <твой IP>` на сервере), и перенеси сервер на
проходящий — часто это UDP 123:

```sh
sed -i 's/:[0-9]*"$/:123"/' /etc/voiceon/server.toml
curl -fsSL https://raw.githubusercontent.com/Arteminariy/voiceOn/main/deploy/install.sh | sh
```

В клиенте тогда сервер указывается как `host:123`.

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
| `core` | общее ядро клиентов: сеть, RNNoise/VAD → Opus, jitter buffer → микшер, проверка подписи релизов |
| `client` | десктоп: звук через cpal, окно egui, трей, автообновление |
| `ffi` | обёртка `core` для Kotlin (UniFFI) → `libvoiceon_ffi.so` |
| `android` | приложение на Kotlin + Compose: звук через AudioRecord/AudioTrack, звонок в foreground-сервисе |

Пакет: `version | kind | room_id | session_id | seq | ciphertext | tag`.
Заголовок открыт, но аутентифицирован; nonce = `session_id ‖ seq`, и каждая сторона
при каждом подключении берёт новый случайный `session_id`, так что nonce не повторяется.

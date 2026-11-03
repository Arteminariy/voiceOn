package io.github.arteminariy.voiceon.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.arteminariy.voiceon.BuildConfig
import io.github.arteminariy.voiceon.Call
import io.github.arteminariy.voiceon.Prefs
import io.github.arteminariy.voiceon.Updater
import io.github.arteminariy.voiceon.core.CallSnapshot
import io.github.arteminariy.voiceon.core.coreVersion
import kotlin.math.log10
import kotlin.math.max
import kotlin.math.roundToInt
import kotlinx.coroutines.delay

private val VAD_RANGE = 0.2f..0.95f

private enum class Screen { Main, Settings }

/** Корень: вход или звонок, плюс настройки поверх. */
@Composable
fun App(call: Call.State, update: Updater.State, onConnect: () -> Unit) {
    val p = LocalPalette.current
    var screen by remember { mutableStateOf(Screen.Main) }
    Box(Modifier.fillMaxSize().background(p.bg).safeDrawingPadding()) {
        when {
            screen == Screen.Settings -> {
                BackHandler { screen = Screen.Main }
                SettingsScreen(call) { screen = Screen.Main }
            }
            call.phase != Call.Phase.Idle -> CallScreen(call) { screen = Screen.Settings }
            else -> LoginScreen(call.error, update, onConnect) { screen = Screen.Settings }
        }
    }
}

/** Снимок состояния сессии раз в 100 мс, пока экран на виду. */
@Composable
private fun rememberSnapshot(): CallSnapshot? {
    var snap by remember { mutableStateOf(Call.session?.snapshot()) }
    LaunchedEffect(Unit) {
        while (true) {
            snap = Call.session?.snapshot()
            delay(100)
        }
    }
    return snap
}

// ---------- вход ----------

@Composable
private fun LoginScreen(error: String?, update: Updater.State, onConnect: () -> Unit, onSettings: () -> Unit) {
    val p = LocalPalette.current
    val context = LocalContext.current
    val prefs = remember { Prefs(context) }
    var server by remember { mutableStateOf(prefs.server) }
    var nick by remember { mutableStateOf(prefs.nick) }
    var pass by remember { mutableStateOf(prefs.passphrase) }
    Column(Modifier.fillMaxSize().imePadding()) {
        Column(
            Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 24.dp, vertical = 16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Logo(28.dp, p.accent)
                Spacer(Modifier.width(8.dp))
                Text("voiceOn", color = p.text, fontSize = 20.sp, fontWeight = FontWeight.SemiBold)
                Spacer(Modifier.weight(1f))
                IconButton(Ph.SLIDERS, p.muted, "Настройки", onSettings)
            }
            UpdateBanner(update)
            Labelled("Сервер") { Input(server, { server = it }, "host:порт", KeyboardType.Uri) }
            Labelled("Ник") { Input(nick, { nick = it }) }
            Labelled("Пароль комнаты") { Input(pass, { pass = it }, password = true) }
            PrimaryButton(Ph.MICROPHONE, "Подключиться") {
                // Люди вставляют адрес как ссылку — отрезаем схему и хвостовой слэш, как десктоп.
                val s = server.trim().removePrefix("https://").removePrefix("http://").removePrefix("udp://").trimEnd('/')
                server = s
                prefs.server = s
                prefs.nick = nick.trim()
                prefs.passphrase = pass
                onConnect()
            }
            if (error != null) ErrorBox(error)
        }
        Text(
            "v${coreVersion()}",
            color = p.dim,
            fontSize = 12.sp,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth().padding(vertical = 10.dp),
        )
    }
}

@Composable
private fun UpdateBanner(update: Updater.State) {
    val p = LocalPalette.current
    val context = LocalContext.current
    val text = when (update) {
        Updater.State.Idle -> return
        is Updater.State.Available -> "Доступна ${update.tag} — Обновить"
        is Updater.State.Downloading -> "Загружаю ${update.tag}…"
        is Updater.State.NeedPermission -> "Разреши установку из voiceOn и нажми ещё раз — Обновить"
        is Updater.State.Installing -> "Устанавливаю ${update.tag}…"
        is Updater.State.Failed -> "Обновление ${update.tag}: ${update.message}. Повторить"
    }
    val clickable = update is Updater.State.Available || update is Updater.State.NeedPermission || update is Updater.State.Failed
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .background(p.accent.copy(alpha = if (p.dark) 0.16f else 0.18f))
            .clickable(enabled = clickable, role = Role.Button) { Updater.update(context) }
            .padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Ph.DOWNLOAD, 18.sp, p.accent)
        Spacer(Modifier.width(8.dp))
        Text(text, color = p.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
    }
}

@Composable
private fun Labelled(label: String, content: @Composable () -> Unit) {
    val p = LocalPalette.current
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(label, color = p.muted, fontSize = 13.sp)
        content()
    }
}

@Composable
private fun Input(value: String, onChange: (String) -> Unit, hint: String? = null, keyboard: KeyboardType = KeyboardType.Text, password: Boolean = false) {
    val p = LocalPalette.current
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        singleLine = true,
        placeholder = hint?.let { { Text(it, color = p.dim) } },
        visualTransformation = if (password) PasswordVisualTransformation() else androidx.compose.ui.text.input.VisualTransformation.None,
        keyboardOptions = KeyboardOptions(
            keyboardType = if (password) KeyboardType.Password else keyboard,
            imeAction = ImeAction.Next,
            autoCorrectEnabled = false,
        ),
        shape = RoundedCornerShape(10.dp),
        colors = OutlinedTextFieldDefaults.colors(
            focusedTextColor = p.text,
            unfocusedTextColor = p.text,
            focusedContainerColor = p.surface,
            unfocusedContainerColor = p.surface,
            focusedBorderColor = p.accent,
            unfocusedBorderColor = p.borderInput,
            cursorColor = p.accent,
        ),
        modifier = Modifier.fillMaxWidth(),
    )
}

@Composable
private fun PrimaryButton(glyph: String, text: String, onClick: () -> Unit) {
    val p = LocalPalette.current
    Row(
        Modifier
            .fillMaxWidth()
            .height(52.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(p.accent)
            .clickable(role = Role.Button, onClick = onClick),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(glyph, 20.sp, p.onAccent)
        Spacer(Modifier.width(8.dp))
        Text(text, color = p.onAccent, fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
    }
}

// ---------- звонок ----------

@Composable
private fun CallScreen(call: Call.State, onSettings: () -> Unit) {
    val p = LocalPalette.current
    val context = LocalContext.current
    val prefs = remember { Prefs(context) }
    val snap = rememberSnapshot()
    val nick = remember { prefs.nick }
    // Громкость, пока палец на слайдере: показываем её, а не устаревший снимок.
    val dragging = remember { mutableStateMapOf<String, Float>() }
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Logo(22.dp, p.accent)
            Spacer(Modifier.width(8.dp))
            Text("voiceOn", color = p.text, fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
            Spacer(Modifier.width(10.dp))
            PingPill(snap?.status, snap?.rttMs, snap?.lossPct ?: 0f)
            Spacer(Modifier.weight(1f))
            IconButton(Ph.SLIDERS, p.muted, "Настройки", onSettings)
        }
        HorizontalDivider(color = p.border)
        val peers = snap?.peers.orEmpty()
        LazyColumn(Modifier.weight(1f).padding(horizontal = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            item {
                Text(
                    "В КОМНАТЕ · ${peers.size + 1}",
                    color = p.muted,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.padding(start = 8.dp, top = 12.dp, bottom = 2.dp),
                )
            }
            item {
                val transmitting = snap?.transmitting == true
                Row(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(p.surface).padding(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Avatar(nick, transmitting)
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Text(
                            buildAnnotatedString {
                                withStyle(SpanStyle(fontWeight = FontWeight.SemiBold, color = p.text)) { append(nick) }
                                withStyle(SpanStyle(color = p.muted)) { append(" (ты)") }
                            },
                            fontSize = 14.sp,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        val db = 20f * log10(max(snap?.micLevel ?: 0f, 1e-5f))
                        LevelBar((db + 60f) / 60f)
                    }
                    Spacer(Modifier.width(12.dp))
                    val (glyph, color) = when {
                        call.micMuted || call.deafened -> Ph.MICROPHONE_SLASH to p.danger
                        transmitting -> Ph.MICROPHONE to p.accent
                        else -> Ph.MICROPHONE to p.muted
                    }
                    Icon(glyph, 20.sp, color)
                }
            }
            if (peers.isEmpty()) {
                item {
                    Text(
                        if (snap == null || snap.status != io.github.arteminariy.voiceon.core.CallStatus.CONNECTED) "Подключаюсь…" else "Пока никого нет",
                        color = p.dim,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth().padding(top = 16.dp),
                    )
                }
            }
            items(peers, key = { it.sessionId.toLong() }) { peer ->
                Row(Modifier.fillMaxWidth().padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Avatar(peer.nick, peer.speaking)
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        Text(peer.nick, color = p.text, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        val vol = dragging[peer.nick] ?: peer.volume
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Icon(if (vol <= 0f) Ph.SPEAKER_SLASH else Ph.SPEAKER_HIGH, 14.sp, p.muted)
                            VolumeSlider(
                                vol,
                                onChange = {
                                    dragging[peer.nick] = it
                                    Call.session?.setVolume(peer.nick, it)
                                },
                                onDone = {
                                    Call.session?.let { s -> prefs.volumes = s.volumes() }
                                    dragging.remove(peer.nick)
                                },
                                modifier = Modifier.weight(1f),
                            )
                            Text("${(vol * 100).roundToInt()}%", color = p.muted, fontSize = 12.sp, textAlign = TextAlign.End, modifier = Modifier.width(44.dp))
                        }
                    }
                }
            }
        }
        HorizontalDivider(color = p.border)
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 16.dp),
            horizontalArrangement = Arrangement.SpaceEvenly,
        ) {
            val (micFill, micFg) = if (call.micMuted) p.roundBtnMuted to p.danger else p.roundBtn to p.text
            RoundButton(
                if (call.micMuted) Ph.MICROPHONE_SLASH else Ph.MICROPHONE, micFill, micFg,
                if (call.micMuted) "Включить микрофон" else "Выключить микрофон",
            ) { Call.setMicMuted(!call.micMuted) }
            val (deafFill, deafFg) = if (call.deafened) p.roundBtnMuted to p.danger else p.roundBtn to p.text
            RoundButton(
                if (call.deafened) Ph.SPEAKER_SLASH else Ph.SPEAKER_HIGH, deafFill, deafFg,
                if (call.deafened) "Включить звук" else "Выключить звук",
            ) { Call.setDeafened(!call.deafened) }
            val (spFill, spFg) = if (call.speaker) p.accent to p.onAccent else p.roundBtn to p.text
            RoundButton(
                Ph.SPEAKER_HIFI, spFill, spFg,
                if (call.speaker) "Громкая связь: выключить" else "Громкая связь",
            ) { Call.setSpeaker(context, !call.speaker) }
            RoundButton(Ph.PHONE_DISCONNECT, p.danger, androidx.compose.ui.graphics.Color.White, "Отключиться") {
                Call.hangup(context)
            }
        }
    }
}

// ---------- настройки ----------

@Composable
private fun SettingsScreen(call: Call.State, onBack: () -> Unit) {
    val p = LocalPalette.current
    val context = LocalContext.current
    val prefs = remember { Prefs(context) }
    val inCall = call.phase == Call.Phase.InCall
    val snap = rememberSnapshot()
    var threshold by remember { mutableFloatStateOf(prefs.vadThreshold) }
    var denoise by remember { mutableStateOf(prefs.denoise) }
    var testTone by remember { mutableStateOf(prefs.testTone) }
    // Пик вероятности речи затухает, как на десктопе.
    var peak by remember { mutableFloatStateOf(0f) }
    val prob = snap?.voiceProb ?: 0f
    LaunchedEffect(snap) { peak = max(prob, peak - 0.06f) }

    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(Ph.ARROW_LEFT, p.text, "Назад", onBack)
            Spacer(Modifier.width(4.dp))
            Text("Настройки", color = p.text, fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
        }
        HorizontalDivider(color = p.border)
        Column(
            Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Row {
                    Text("Голосовая активация", color = p.muted, fontSize = 13.sp)
                    Spacer(Modifier.weight(1f))
                    Text("порог %.2f".format(threshold), color = p.dim, fontSize = 12.sp)
                }
                VadBar(
                    prob, peak, threshold, VAD_RANGE,
                    onChange = {
                        threshold = it
                        Call.session?.setVadThreshold(it)
                    },
                    onDone = { prefs.vadThreshold = threshold },
                )
                Text(
                    if (inCall) {
                        "Двигай белую черту: всё, что громче неё, уходит в эфир. Скажи что-нибудь, чтобы проверить."
                    } else {
                        "Двигай белую черту: всё, что громче неё, уходит в эфир. Уровень голоса виден во время звонка."
                    },
                    color = p.dim,
                    fontSize = 12.sp,
                )
            }
            ToggleCard("Шумодав", "убирает клавиатуру и вентиляторы", denoise) {
                denoise = it
                prefs.denoise = it
                Call.session?.setDenoise(it)
            }
            if (BuildConfig.DEBUG) {
                ToggleCard("Тестовый тон (отладка)", "вместо микрофона, со следующего звонка", testTone) {
                    testTone = it
                    prefs.testTone = it
                }
            }
        }
        Text(
            "voiceOn v${coreVersion()}",
            color = p.dim,
            fontSize = 12.sp,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth().padding(vertical = 10.dp),
        )
    }
}

@Composable
private fun ToggleCard(title: String, subtitle: String, on: Boolean, onChange: (Boolean) -> Unit) {
    val p = LocalPalette.current
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).background(p.surface).padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(title, color = p.text, fontSize = 14.sp)
            Text(subtitle, color = p.dim, fontSize = 12.sp)
        }
        Toggle(on, onChange)
    }
}

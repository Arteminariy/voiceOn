package io.github.arteminariy.voiceon.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.arteminariy.voiceon.core.CallStatus
import kotlin.math.roundToInt

@Composable
fun Icon(glyph: String, size: TextUnit, color: Color, modifier: Modifier = Modifier) {
    Text(glyph, fontFamily = PhosphorFont, fontSize = size, color = color, modifier = modifier)
}

/** Логотип: пять столбиков волны (штрихи из макета, viewBox 24x24). */
@Composable
fun Logo(size: Dp, color: Color) {
    Canvas(Modifier.size(size)) {
        val k = this.size.width / 24f
        for ((x, y0, y1) in listOf(Triple(4f, 10f, 14f), Triple(8f, 7f, 17f), Triple(12f, 4f, 20f), Triple(16f, 7f, 17f), Triple(20f, 10f, 14f))) {
            drawLine(color, Offset(x * k, y0 * k), Offset(x * k, y1 * k), strokeWidth = 2f * k, cap = StrokeCap.Round)
        }
    }
}

/** Детерминированный цвет аватара по хешу ника (FNV-1a, как на десктопе). */
fun avatarColor(nick: String, p: Palette): Color {
    var h = 0x811c9dc5.toInt()
    for (b in nick.toByteArray(Charsets.UTF_8)) {
        h = h xor (b.toInt() and 0xff)
        h *= 0x01000193
    }
    return p.avatars[(h.toUInt() % p.avatars.size.toUInt()).toInt()]
}

/** Круг с буквой ника; у говорящего — зелёное кольцо. */
@Composable
fun Avatar(nick: String, speaking: Boolean) {
    val p = LocalPalette.current
    Box(Modifier.size(44.dp), contentAlignment = Alignment.Center) {
        Canvas(Modifier.size(44.dp)) {
            if (speaking) drawCircle(p.accent, radius = 21.dp.toPx(), style = Stroke(2.dp.toPx()))
            drawCircle(avatarColor(nick, p), radius = 18.dp.toPx())
        }
        val initial = nick.firstOrNull()?.uppercase() ?: "?"
        Text(initial, color = p.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
    }
}

/** Круглая кнопка нижней панели. */
@Composable
fun RoundButton(glyph: String, fill: Color, fg: Color, label: String, onClick: () -> Unit) {
    Box(
        Modifier
            .size(60.dp)
            .clip(CircleShape)
            .background(fill)
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(glyph, 24.sp, fg)
    }
}

/** Иконка-кнопка без фона (настройки, «назад»). */
@Composable
fun IconButton(glyph: String, color: Color, label: String, onClick: () -> Unit) {
    Box(
        Modifier
            .size(44.dp)
            .clip(RoundedCornerShape(10.dp))
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(glyph, 22.sp, color)
    }
}

/** Тонкая полоса уровня (0..1). */
@Composable
fun LevelBar(frac: Float, modifier: Modifier = Modifier) {
    val p = LocalPalette.current
    Canvas(modifier.fillMaxWidth().height(4.dp)) {
        val r = CornerRadius(2.dp.toPx())
        drawRoundRect(p.track, cornerRadius = r)
        drawRoundRect(p.accent, size = Size(size.width * frac.coerceIn(0f, 1f), size.height), cornerRadius = r)
    }
}

/** Слайдер громкости 0..2 (0–200%) с шагом 5%; двойное касание — 100%. */
@Composable
fun VolumeSlider(value: Float, onChange: (Float) -> Unit, onDone: () -> Unit, modifier: Modifier = Modifier) {
    val p = LocalPalette.current
    val change by rememberUpdatedState(onChange)
    val done by rememberUpdatedState(onDone)
    Canvas(
        modifier
            .height(32.dp)
            .pointerInput(Unit) {
                val inset = 9.dp.toPx()
                fun at(x: Float) = (((x - inset) / (size.width - 2 * inset)) * 2f).coerceIn(0f, 2f).let { (it * 20f).roundToInt() / 20f }
                detectTapGestures(onDoubleTap = { change(1f); done() }, onTap = { change(at(it.x)); done() })
            }
            .pointerInput(Unit) {
                val inset = 9.dp.toPx()
                fun at(x: Float) = (((x - inset) / (size.width - 2 * inset)) * 2f).coerceIn(0f, 2f).let { (it * 20f).roundToInt() / 20f }
                detectHorizontalDragGestures(onDragEnd = { done() }) { c, _ -> change(at(c.position.x)) }
            },
    ) {
        val inset = 9.dp.toPx()
        val y = size.height / 2
        val l = inset
        val r = size.width - inset
        val x = l + (r - l) * (value / 2f).coerceIn(0f, 1f)
        val h = 2.dp.toPx()
        drawRoundRect(p.track, Offset(l, y - h), Size(r - l, 2 * h), CornerRadius(h))
        drawRoundRect(p.volFill, Offset(l, y - h), Size(x - l, 2 * h), CornerRadius(h))
        drawCircle(p.text, radius = 8.dp.toPx(), center = Offset(x, y))
    }
}

/** Переключатель 44x26. */
@Composable
fun Toggle(on: Boolean, onChange: (Boolean) -> Unit) {
    val p = LocalPalette.current
    Canvas(
        Modifier
            .size(48.dp, 28.dp)
            .clip(RoundedCornerShape(14.dp))
            .clickable(role = Role.Switch) { onChange(!on) },
    ) {
        drawRoundRect(if (on) p.accent else p.track, cornerRadius = CornerRadius(size.height / 2))
        val r = 10.dp.toPx()
        val cx = if (on) size.width - size.height / 2 else size.height / 2
        drawCircle(if (on) p.onAccent else p.muted, radius = r, center = Offset(cx, size.height / 2))
    }
}

/**
 * Полоса голосовой активации: текущая вероятность речи, затухающий пик и
 * перетаскиваемая черта порога (в пределах `range`).
 */
@Composable
fun VadBar(prob: Float, peak: Float, threshold: Float, range: ClosedFloatingPointRange<Float>, onChange: (Float) -> Unit, onDone: () -> Unit) {
    val p = LocalPalette.current
    val change by rememberUpdatedState(onChange)
    val done by rememberUpdatedState(onDone)
    Canvas(
        Modifier
            .fillMaxWidth()
            .height(32.dp)
            .pointerInput(Unit) {
                detectTapGestures { change((it.x / size.width).coerceIn(range)); done() }
            }
            .pointerInput(Unit) {
                detectHorizontalDragGestures(onDragEnd = { done() }) { c, _ -> change((c.position.x / size.width).coerceIn(range)) }
            },
    ) {
        val r = CornerRadius(6.dp.toPx())
        drawRoundRect(p.surface, cornerRadius = r)
        val inner = 1.dp.toPx()
        val w = size.width - 2 * inner
        val h = size.height - 2 * inner
        val ir = CornerRadius(5.dp.toPx())
        drawRoundRect(p.vadPeak, Offset(inner, inner), Size(w * peak.coerceIn(0f, 1f), h), ir)
        drawRoundRect(p.accent, Offset(inner, inner), Size(w * prob.coerceIn(0f, 1f), h), ir)
        drawRoundRect(p.borderInput, cornerRadius = r, style = Stroke(1.dp.toPx()))
        val x = size.width * threshold.coerceIn(0f, 1f)
        val lw = 3.dp.toPx()
        drawRoundRect(p.text, Offset(x - lw / 2, -2.dp.toPx()), Size(lw, size.height + 4.dp.toPx()), CornerRadius(1.dp.toPx()))
    }
}

/** Значок уровня связи: четыре столбика. */
@Composable
fun SignalIcon(color: Color) {
    Canvas(Modifier.size(14.dp)) {
        val k = size.width / 24f
        for ((x, top) in listOf(5f to 15f, 10f to 12f, 15f to 9f, 20f to 6f)) {
            drawLine(color, Offset(x * k, top * k), Offset(x * k, 18f * k), strokeWidth = 2.4f * k)
        }
    }
}

/** Пилюля с пингом: цвет по качеству связи; при подключении — жёлтая с текстом статуса. */
@Composable
fun PingPill(status: CallStatus?, rtt: Float?, loss: Float) {
    val p = LocalPalette.current
    val (text, color) = when {
        status == null || status == CallStatus.CONNECTING -> "Подключение…" to p.warn
        status == CallStatus.RECONNECTING -> "Переподключение…" to p.warn
        rtt == null -> "в сети" to p.accent
        else -> {
            val c = if (rtt < 80) p.accent else if (rtt < 150) p.warn else p.danger
            val t = if (loss > 0) "${rtt.roundToInt()} мс · ${loss.roundToInt()}%" else "${rtt.roundToInt()} мс"
            t to c
        }
    }
    Row(
        Modifier
            .clip(RoundedCornerShape(50))
            .background(color.copy(alpha = if (p.dark) 0.16f else 0.18f))
            .padding(horizontal = 10.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        SignalIcon(color)
        Text(text, color = color, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
fun ErrorBox(text: String, modifier: Modifier = Modifier) {
    val p = LocalPalette.current
    Row(
        modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .background(p.errBg)
            .padding(12.dp),
    ) {
        Icon(Ph.WARNING_CIRCLE, 18.sp, p.errText)
        Spacer(Modifier.width(8.dp))
        Text(text, color = p.errText, fontSize = 13.sp)
    }
}

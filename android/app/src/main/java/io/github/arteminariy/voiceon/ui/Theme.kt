package io.github.arteminariy.voiceon.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import io.github.arteminariy.voiceon.R

/** Палитра десктопного клиента (client/src/ui/theme.rs). */
@Immutable
data class Palette(
    val dark: Boolean,
    val bg: Color,
    val surface: Color,
    val border: Color,
    val borderInput: Color,
    val text: Color,
    val muted: Color,
    val dim: Color,
    val accent: Color,
    val onAccent: Color,
    val danger: Color,
    val warn: Color,
    val track: Color,
    val roundBtn: Color,
    val roundBtnMuted: Color,
    val errBg: Color,
    val errText: Color,
    val volFill: Color,
    val vadPeak: Color,
    val avatars: List<Color>,
)

private fun c(rgb: Long) = Color(0xFF000000 or rgb)

val DarkPalette = Palette(
    dark = true,
    bg = c(0x15171B),
    surface = c(0x1B1E23),
    border = c(0x262A31),
    borderInput = c(0x2E333B),
    text = c(0xE8EAED),
    muted = c(0x9AA0A8),
    dim = c(0x80868F),
    accent = c(0x3DDC84),
    onAccent = c(0x0D1F14),
    danger = c(0xE5484D),
    warn = c(0xF5A524),
    track = c(0x2A2E35),
    roundBtn = c(0x262A31),
    roundBtnMuted = c(0x3A2224),
    errBg = c(0x2A1A1C),
    errText = c(0xFF9EA1),
    volFill = c(0xC9CDD3),
    vadPeak = c(0x1F3A2A),
    avatars = listOf(c(0x3A4A7A), c(0x6B4A2E), c(0x2E5B55), c(0x6A3A5E), c(0x55602F), c(0x6B3A3A), c(0x3A5F7A)),
)

val LightPalette = Palette(
    dark = false,
    bg = c(0xF6F7F9),
    surface = c(0xFFFFFF),
    border = c(0xE1E4E8),
    borderInput = c(0xD3D7DD),
    text = c(0x16181C),
    muted = c(0x5F6670),
    dim = c(0x7A828C),
    accent = c(0x1F9D57),
    onAccent = c(0xFFFFFF),
    danger = c(0xE5484D),
    warn = c(0xB7791F),
    track = c(0xE1E4E8),
    roundBtn = c(0xE1E4E8),
    roundBtnMuted = c(0xFBDADB),
    errBg = c(0xFCE8E9),
    errText = c(0xB4232A),
    volFill = c(0x5F6670),
    vadPeak = c(0xB9E3CB),
    avatars = listOf(c(0xC9D4F5), c(0xF2D9C4), c(0xC5E6E1), c(0xEBCDE0), c(0xDCE5BE), c(0xF0CCCC), c(0xC4DDF0)),
)

val LocalPalette = staticCompositionLocalOf { DarkPalette }

/** Иконки Phosphor (как на десктопе), подмножество шрифта в res/font/phosphor.ttf. */
val PhosphorFont = FontFamily(Font(R.font.phosphor))

object Ph {
    const val ARROW_LEFT = ""
    const val DOWNLOAD = ""
    const val HEADPHONES = ""
    const val MICROPHONE = ""
    const val MICROPHONE_SLASH = ""
    const val PHONE_DISCONNECT = ""
    const val SLIDERS = ""
    const val SPEAKER_HIGH = ""
    const val SPEAKER_SLASH = ""
    const val WARNING_CIRCLE = ""
    const val SPEAKER_HIFI = ""
}

@Composable
fun VoiceTheme(content: @Composable () -> Unit) {
    val p = if (isSystemInDarkTheme()) DarkPalette else LightPalette
    val scheme = if (p.dark) {
        darkColorScheme(
            primary = p.accent, onPrimary = p.onAccent, background = p.bg, onBackground = p.text,
            surface = p.surface, onSurface = p.text, onSurfaceVariant = p.muted, outline = p.borderInput,
            error = p.danger,
        )
    } else {
        lightColorScheme(
            primary = p.accent, onPrimary = p.onAccent, background = p.bg, onBackground = p.text,
            surface = p.surface, onSurface = p.text, onSurfaceVariant = p.muted, outline = p.borderInput,
            error = p.danger,
        )
    }
    CompositionLocalProvider(LocalPalette provides p) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}

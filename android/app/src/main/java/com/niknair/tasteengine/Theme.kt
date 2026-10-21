package com.niknair.tasteengine

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * Taste Engine design tokens, mirroring frontend/style.css (:root + light).
 * Flat surfaces, zero radius, crimson field with obsidian work surfaces.
 */
data class TasteTokens(
    val field: Color,
    val fieldMuted: Color,
    val fieldLine: Color,
    val bg2: Color,
    val canvas: Color,
    val surface: Color,
    val surface2: Color,
    val surface3: Color,
    val line: Color,
    val lineSoft: Color,
    val lineStrong: Color,
    val text: Color,
    val muted: Color,
    val accent: Color,
    val accentStrong: Color,
    val accentDim: Color,
    val danger: Color,
    val ok: Color,
    val onAccent: Color,
)

val DarkTaste = TasteTokens(
    field = Color(0xFFC80044),
    fieldMuted = Color(0xFFFFDDE6),
    fieldLine = Color(0x57FFFFFF),
    bg2 = Color(0xFFA80038),
    canvas = Color(0xFF121012),
    surface = Color(0xFF1A1718),
    surface2 = Color(0xFF272124),
    surface3 = Color(0xFF302A2D),
    line = Color(0xFF3A3236),
    lineSoft = Color(0xFF2D272A),
    lineStrong = Color(0xFF554246),
    text = Color(0xFFFDF6F6),
    muted = Color(0xFFE8B8C8),
    accent = Color(0xFFC90044),
    accentStrong = Color(0xFFE4628B),
    accentDim = Color(0xFFA13655),
    danger = Color(0xFFEF5B57),
    ok = Color(0xFF8FBF9F),
    onAccent = Color(0xFFFDF6F6),
)

val LightTaste = TasteTokens(
    field = Color(0xFFF3C9D2),
    fieldMuted = Color(0xFF9C223F),
    fieldLine = Color(0x597A122D),
    bg2 = Color(0xFFE8B2BD),
    canvas = Color(0xFFFFFFFF),
    surface = Color(0xFFFFFFFF),
    surface2 = Color(0xFFF4EFED),
    surface3 = Color(0xFFE9E3E0),
    line = Color(0xFFD9C3C9),
    lineSoft = Color(0xFFE6D5D9),
    lineStrong = Color(0xFFC2A2AB),
    text = Color(0xFF2A1018),
    muted = Color(0xFF8A4C5C),
    accent = Color(0xFFC2274E),
    accentStrong = Color(0xFF8F1F3B),
    accentDim = Color(0xFFD97FA0),
    danger = Color(0xFFC23E4C),
    ok = Color(0xFF5D9368),
    onAccent = Color(0xFFFDF6F6),
)

val LocalTaste = staticCompositionLocalOf { DarkTaste }

/** Self-hosted Archivo (SIL OFL 1.1) — the same face the web app serves. */
val AppFont = FontFamily(
    Font(R.font.archivo_regular),
    Font(R.font.archivo_regular, FontWeight.Medium),
    Font(R.font.archivo_semibold, FontWeight.SemiBold),
    Font(R.font.archivo_semibold, FontWeight.Bold),
    Font(R.font.archivo_extralight, FontWeight.ExtraLight),
)

private val Base = Typography()

private fun tasteTypography(): Typography = Typography(
    displayLarge = Base.displayLarge.copy(fontFamily = AppFont),
    displayMedium = Base.displayMedium.copy(fontFamily = AppFont),
    displaySmall = Base.displaySmall.copy(fontFamily = AppFont),
    headlineLarge = Base.headlineLarge.copy(fontFamily = AppFont, fontWeight = FontWeight.SemiBold),
    headlineMedium = Base.headlineMedium.copy(fontFamily = AppFont, fontWeight = FontWeight.SemiBold),
    headlineSmall = Base.headlineSmall.copy(fontFamily = AppFont, fontWeight = FontWeight.SemiBold),
    titleLarge = Base.titleLarge.copy(fontFamily = AppFont, fontWeight = FontWeight.SemiBold),
    titleMedium = Base.titleMedium.copy(fontFamily = AppFont, fontWeight = FontWeight.SemiBold),
    titleSmall = Base.titleSmall.copy(fontFamily = AppFont, fontWeight = FontWeight.SemiBold),
    bodyLarge = Base.bodyLarge.copy(fontFamily = AppFont),
    bodyMedium = Base.bodyMedium.copy(fontFamily = AppFont),
    bodySmall = Base.bodySmall.copy(fontFamily = AppFont),
    labelLarge = Base.labelLarge.copy(fontFamily = AppFont),
    labelMedium = Base.labelMedium.copy(fontFamily = AppFont),
    labelSmall = Base.labelSmall.copy(fontFamily = AppFont, letterSpacing = 1.2.sp),
)

@Composable
fun TasteTheme(darkTheme: Boolean = true, content: @Composable () -> Unit) {
    val t = if (darkTheme) DarkTaste else LightTaste
    val scheme = if (darkTheme) {
        darkColorScheme(
            primary = t.accentStrong, onPrimary = t.onAccent,
            secondary = t.accent, onSecondary = t.onAccent,
            background = t.canvas, onBackground = t.text,
            surface = t.surface, onSurface = t.text,
            surfaceVariant = t.surface2, onSurfaceVariant = t.muted,
            outline = t.lineStrong, outlineVariant = t.line,
            error = t.danger, onError = t.onAccent,
        )
    } else {
        lightColorScheme(
            primary = t.accent, onPrimary = t.onAccent,
            secondary = t.accentStrong, onSecondary = t.onAccent,
            background = t.canvas, onBackground = t.text,
            surface = t.surface, onSurface = t.text,
            surfaceVariant = t.surface2, onSurfaceVariant = t.muted,
            outline = t.lineStrong, outlineVariant = t.line,
            error = t.danger, onError = t.onAccent,
        )
    }
    CompositionLocalProvider(LocalTaste provides t) {
        MaterialTheme(
            colorScheme = scheme,
            typography = tasteTypography(),
            shapes = Shapes(
                extraSmall = RoundedCornerShape(0.dp),
                small = RoundedCornerShape(0.dp),
                medium = RoundedCornerShape(0.dp),
                large = RoundedCornerShape(0.dp),
                extraLarge = RoundedCornerShape(0.dp),
            ),
            content = content,
        )
    }
}
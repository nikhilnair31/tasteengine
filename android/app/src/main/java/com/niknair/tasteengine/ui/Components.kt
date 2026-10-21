package com.niknair.tasteengine.ui

import android.content.Context
import android.net.Uri
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.staggeredgrid.LazyVerticalStaggeredGrid
import androidx.compose.foundation.lazy.staggeredgrid.StaggeredGridCells
import androidx.compose.foundation.lazy.staggeredgrid.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.foundation.Canvas
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.niknair.tasteengine.LocalTaste
import com.niknair.tasteengine.AppFont
import com.niknair.tasteengine.vm.VaultVm
import java.io.File

/** Small uppercase mark: uses accent-strong rose on dark surfaces. */
@Composable
fun Mark(text: String, color: Color? = null, modifier: Modifier = Modifier) {
    val t = LocalTaste.current
    Text(
        text = text.uppercase(),
        color = color ?: t.accentStrong,
        fontSize = 10.sp,
        fontFamily = AppFont,
        fontWeight = FontWeight.SemiBold,
        letterSpacing = 1.2.sp,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        modifier = modifier,
    )
}

/** Section headers inside work surfaces: muted, never rose (rose = interactive). */
@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    val t = LocalTaste.current
    Text(
        text = text.uppercase(),
        color = t.muted,
        fontSize = 11.sp,
        fontFamily = AppFont,
        fontWeight = FontWeight.SemiBold,
        letterSpacing = 1.6.sp,
        modifier = modifier,
    )
}

@Composable
fun FlatButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    ghost: Boolean = false,
    danger: Boolean = false,
    enabled: Boolean = true,
) {
    val t = LocalTaste.current
    val border = when {
        danger -> t.danger
        ghost -> t.lineStrong
        else -> Color(0xCCFDF6F6)
    }
    val fg = when {
        !enabled -> t.muted.copy(alpha = 0.5f)
        danger -> t.danger
        else -> t.text
    }
    Box(
        modifier = modifier
            .background(if (ghost) Color.Transparent else t.surface)
            .border(1.dp, border, RoundedCornerShape(0.dp))
            .clickable(enabled = enabled) { onClick() }
            .padding(horizontal = 16.dp, vertical = 10.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = text,
            color = fg,
            fontSize = 13.sp,
            fontFamily = AppFont,
            fontWeight = FontWeight.Medium,
            letterSpacing = 0.4.sp,
        )
    }
}

/** Dark print chip (square); active = white slab with accent text. */
@Composable
fun Chip(
    label: String,
    active: Boolean,
    count: Int? = null,
    onClick: () -> Unit,
) {
    val t = LocalTaste.current
    val bg = if (active) Color(0xFFFDF6F6) else t.surface
    val fg = if (active) t.accent else t.text
    Row(
        modifier = Modifier
            .background(bg, RoundedCornerShape(0.dp))
            .border(1.dp, if (active) Color.Transparent else t.lineStrong, RoundedCornerShape(0.dp))
            .clickable { onClick() }
            .padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, color = fg, fontSize = 12.sp, fontFamily = AppFont, maxLines = 1)
        if (count != null && count > 0) {
            Spacer(Modifier.width(6.dp))
            Text(
                count.toString(),
                color = if (active) t.accent else t.accentStrong,
                fontSize = 10.sp,
                fontFamily = AppFont,
            )
        }
    }
}

/** Crimson input block with field-muted placeholder (mirrors web inputs). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TasteField(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
    modifier: Modifier = Modifier,
    password: Boolean = false,
    singleLine: Boolean = true,
    minLines: Int = 1,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
    keyboardActions: KeyboardActions = KeyboardActions.Default,
    trailing: (@Composable () -> Unit)? = null,
    leading: (@Composable () -> Unit)? = null,
    textSize: TextUnit = 14.sp,
) {
    val t = LocalTaste.current
    TextField(
        value = value,
        onValueChange = onValueChange,
        modifier = modifier,
        placeholder = {
            Text(placeholder, color = t.fieldMuted, fontSize = 13.sp, fontFamily = AppFont, maxLines = 1)
        },
        singleLine = singleLine,
        minLines = minLines,
        leadingIcon = leading,
        trailingIcon = trailing,
        visualTransformation = if (password) PasswordVisualTransformation() else VisualTransformation.None,
        keyboardOptions = keyboardOptions,
        keyboardActions = keyboardActions,
        textStyle = TextStyle(fontFamily = AppFont, fontSize = textSize, color = t.text),
        colors = TextFieldDefaults.colors(
            focusedContainerColor = t.bg2,
            unfocusedContainerColor = t.bg2,
            disabledContainerColor = t.bg2,
            focusedTextColor = t.text,
            unfocusedTextColor = t.text,
            cursorColor = t.accentStrong,
            focusedIndicatorColor = Color.Transparent,
            unfocusedIndicatorColor = Color.Transparent,
            focusedPlaceholderColor = t.fieldMuted,
            unfocusedPlaceholderColor = t.fieldMuted,
        ),
    )
}

/** Quiet search row: obsidian inset inside the bar, hairline rose focus ring. */
@Composable
fun SearchRow(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
    focused: Boolean,
    onFocusChange: (Boolean) -> Unit,
    onSubmit: () -> Unit,
    onClear: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val t = LocalTaste.current
    Row(
        modifier
            .background(t.canvas)
            .border(1.dp, if (focused) t.accentStrong else Color.Transparent, RoundedCornerShape(0.dp))
            .padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            Icons.Default.Search,
            contentDescription = null,
            tint = t.muted,
            modifier = Modifier.size(17.dp),
        )
        Spacer(Modifier.width(9.dp))
        BasicTextField(
            value = value,
            onValueChange = onValueChange,
            singleLine = true,
            textStyle = TextStyle(fontFamily = AppFont, fontSize = 15.sp, color = t.text),
            cursorBrush = SolidColor(t.accentStrong),
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions = KeyboardActions(onSearch = { onSubmit() }),
            modifier = Modifier
                .weight(1f)
                .onFocusChanged { onFocusChange(it.isFocused) },
            decorationBox = { inner ->
                Box(contentAlignment = Alignment.CenterStart) {
                    if (value.isEmpty()) {
                        Text(
                            placeholder,
                            color = t.muted,
                            fontSize = 14.sp,
                            fontFamily = AppFont,
                            maxLines = 1,
                        )
                    }
                    inner()
                }
            },
        )
        if (value.isNotEmpty()) {
            Spacer(Modifier.width(6.dp))
            Box(
                Modifier
                    .size(30.dp)
                    .clickable { onClear() },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    Icons.Default.Close,
                    contentDescription = "clear",
                    tint = t.muted,
                    modifier = Modifier.size(15.dp),
                )
            }
        }
    }
}

/** Skeleton shimmer for covers that are still being pulled. */
@Composable
fun ShimmerBox(modifier: Modifier = Modifier) {
    val t = LocalTaste.current
    val transition = rememberInfiniteTransition(label = "shimmer")
    val phase by transition.animateFloat(
        initialValue = 0f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(tween(1300, easing = LinearEasing)),
        label = "phase",
    )
    val w = 600f
    val x = -w + phase * w * 2f
    Box(
        modifier = modifier.background(
            Brush.linearGradient(
                colors = listOf(t.surface2, t.surface3, t.surface2),
                start = Offset(x, 0f),
                end = Offset(x + w, 240f),
            )
        )
    )
}

/** Soft pulsing label ("pulling media"). */
@Composable
fun PulseText(text: String, color: Color, modifier: Modifier = Modifier) {
    val transition = rememberInfiniteTransition(label = "pulse")
    val alpha by transition.animateFloat(
        initialValue = 0.35f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(tween(800), RepeatMode.Reverse),
        label = "alpha",
    )
    Text(
        text = text,
        color = color.copy(alpha = alpha),
        fontSize = 10.sp,
        fontFamily = AppFont,
        letterSpacing = 1.sp,
        modifier = modifier,
    )
}

/** Sliders glyph: three rails with knobs — the tools sheet button. */
@Composable
fun SlidersGlyph(color: Color, modifier: Modifier = Modifier) {
    Canvas(modifier.size(20.dp)) {
        val s = size.minDimension
        val sw = s * 0.095f
        drawLine(color, Offset(s * 0.10f, s * 0.26f), Offset(s * 0.90f, s * 0.26f), sw, StrokeCap.Round)
        drawLine(color, Offset(s * 0.10f, s * 0.50f), Offset(s * 0.90f, s * 0.50f), sw, StrokeCap.Round)
        drawLine(color, Offset(s * 0.10f, s * 0.74f), Offset(s * 0.90f, s * 0.74f), sw, StrokeCap.Round)
        drawLine(color, Offset(s * 0.34f, s * 0.13f), Offset(s * 0.34f, s * 0.39f), sw * 2.1f, StrokeCap.Round)
        drawLine(color, Offset(s * 0.66f, s * 0.37f), Offset(s * 0.66f, s * 0.63f), sw * 2.1f, StrokeCap.Round)
        drawLine(color, Offset(s * 0.44f, s * 0.61f), Offset(s * 0.44f, s * 0.87f), sw * 2.1f, StrokeCap.Round)
    }
}

/** Staggered grid skeleton — posts loading. */
@Composable
fun ShimmerGrid(columns: Int, modifier: Modifier = Modifier) {
    LazyVerticalStaggeredGrid(
        columns = StaggeredGridCells.Fixed(columns),
        modifier = modifier.fillMaxSize(),
        horizontalArrangement = Arrangement.spacedBy(3.dp),
        verticalItemSpacing = 3.dp,
    ) {
        items(15) { i ->
            ShimmerBox(
                Modifier
                    .fillMaxWidth()
                    .height(
                        when (i % 3) {
                            0 -> 232.dp
                            1 -> 148.dp
                            else -> 300.dp
                        },
                    ),
            )
        }
    }
}

/** List skeleton rows — followups / trash / cleanup loading. */
@Composable
fun ShimmerList(rows: Int = 6, thumb: Dp = 64.dp, modifier: Modifier = Modifier) {
    val t = LocalTaste.current
    Column(
        modifier
            .fillMaxSize()
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        repeat(rows) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .background(t.surface)
                    .border(1.dp, t.line, RoundedCornerShape(0.dp))
                    .padding(10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                ShimmerBox(Modifier.size(thumb))
                Spacer(Modifier.width(10.dp))
                Column(Modifier.weight(1f)) {
                    ShimmerBox(Modifier.fillMaxWidth(0.72f).height(14.dp))
                    Spacer(Modifier.height(9.dp))
                    ShimmerBox(Modifier.fillMaxWidth(0.45f).height(11.dp))
                }
            }
        }
    }
}

@Composable
fun BusyBlock(message: String, modifier: Modifier = Modifier) {
    val t = LocalTaste.current
    Box(modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            CircularProgressIndicator(
                color = t.accentStrong,
                strokeWidth = 2.dp,
                modifier = Modifier.size(22.dp),
            )
            Spacer(Modifier.height(12.dp))
            Text(message, color = t.muted, fontSize = 12.sp, fontFamily = AppFont)
        }
    }
}

@Composable
fun NoteBlock(message: String, modifier: Modifier = Modifier, retry: (() -> Unit)? = null) {
    val t = LocalTaste.current
    Box(modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = message,
                color = t.muted,
                fontSize = 13.sp,
                fontFamily = AppFont,
                textAlign = TextAlign.Center,
                modifier = Modifier.padding(horizontal = 34.dp),
            )
            if (retry != null) {
                FlatButton("retry", onClick = retry, ghost = true)
            }
        }
    }
}

/** Copy a content:// uri into a cache file for multipart upload. */
fun uriToCache(context: Context, uri: Uri): File {
    val mime = context.contentResolver.getType(uri) ?: "image/jpeg"
    val ext = when {
        mime.contains("png") -> ".png"
        mime.contains("webp") -> ".webp"
        mime.contains("gif") -> ".gif"
        else -> ".jpg"
    }
    val f = File(context.cacheDir, "note_${System.currentTimeMillis()}$ext")
    context.contentResolver.openInputStream(uri)?.use { ins ->
        f.outputStream().use { outs -> ins.copyTo(outs) }
    } ?: throw IllegalStateException("cannot read image")
    return f
}

// ---------------------------------------------------------------- toggles

/** Square toggle — no rounded track, no circular thumb. */
@Composable
fun SquareSwitch(checked: Boolean, onCheckedChange: (Boolean) -> Unit) {
    val t = LocalTaste.current
    val thumb by animateDpAsState(if (checked) 20.dp else 0.dp, label = "thumb")
    Box(
        Modifier
            .size(width = 46.dp, height = 26.dp)
            .background(if (checked) t.accent else t.surface2)
            .border(1.dp, if (checked) t.accentStrong else t.lineStrong, RoundedCornerShape(0.dp))
            .clickable { onCheckedChange(!checked) },
    ) {
        Box(
            Modifier
                .padding(start = 3.dp, top = 3.dp)
                .offset(x = thumb)
                .size(18.dp)
                .background(if (checked) Color(0xFFFDF6F6) else t.muted),
        )
    }
}

/** Compact flat icon slot for the top bars (38dp touch target). */
@Composable
fun ToolIcon(
    active: Boolean = false,
    boxed: Boolean = false,
    onClick: () -> Unit,
    desc: String? = null,
    content: @Composable () -> Unit,
) {
    val t = LocalTaste.current
    Box(
        Modifier
            .size(38.dp)
            .then(
                if (boxed) {
                    Modifier
                        .background(t.surface2)
                        .border(1.dp, if (active) t.accentStrong else t.line)
                } else {
                    Modifier
                },
            )
            .semantics { if (desc != null) contentDescription = desc }
            .clickable { onClick() },
        contentAlignment = Alignment.Center,
    ) {
        content()
    }
}

/** Funnel glyph for the chips toggle (no material equivalent in icons-core). */
@Composable
fun FunnelGlyph(tint: Color, modifier: Modifier = Modifier) {
    Canvas(modifier.size(19.dp)) {
        val w = size.width
        val h = size.height
        val p = Path().apply {
            moveTo(w * 0.08f, h * 0.12f)
            lineTo(w * 0.92f, h * 0.12f)
            lineTo(w * 0.60f, h * 0.50f)
            lineTo(w * 0.60f, h * 0.90f)
            lineTo(w * 0.40f, h * 0.90f)
            lineTo(w * 0.40f, h * 0.50f)
            close()
        }
        drawPath(p, tint)
    }
}

/** Grid-pattern glyph that reflects the column count (1 = one big square, 3 = 3x3). */
@Composable
fun GridGlyph(columns: Int, tint: Color, modifier: Modifier = Modifier) {
    Canvas(modifier.size(19.dp)) {
        val n = columns.coerceIn(1, 3)
        val gap = 2.2f.dp.toPx()
        val cell = (size.minDimension - gap * (n - 1)) / n
        for (r in 0 until n) {
            for (c in 0 until n) {
                drawRect(
                    color = tint,
                    topLeft = Offset(c * (cell + gap), r * (cell + gap)),
                    size = Size(cell, cell),
                )
            }
        }
    }
}

/** Contrast-disc glyph: hemisphere filled on the side of the active theme. */
@Composable
fun ThemeGlyph(dark: Boolean, tint: Color, modifier: Modifier = Modifier) {
    Canvas(modifier.size(19.dp)) {
        val r = size.minDimension / 2f - 1.4f.dp.toPx()
        drawCircle(color = tint, radius = r, center = center, style = Stroke(width = 1.6f.dp.toPx()))
        drawArc(
            color = tint,
            startAngle = 90f,
            sweepAngle = if (dark) 180f else -180f,
            useCenter = true,
            topLeft = Offset(center.x - r, center.y - r),
            size = Size(r * 2f, r * 2f),
        )
    }
}

/** Grid-size flip button. Taps cycle states; no menus. */
@Composable
fun ViewControls(vm: VaultVm, modifier: Modifier = Modifier) {
    val t = LocalTaste.current
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        ToolIcon(onClick = { vm.cycleGridColumns() }, desc = "grid size") {
            GridGlyph(vm.gridColumns, t.text)
        }
    }
}

/** External-link glyph: an arrow leaving an open box. */
@Composable
fun ExternalGlyph(color: Color, modifier: Modifier = Modifier) {
    Canvas(modifier.size(20.dp)) {
        val s = size.minDimension
        val sw = s * 0.105f
        val box = Path()
        box.moveTo(s * 0.44f, s * 0.26f)
        box.lineTo(s * 0.15f, s * 0.26f)
        box.lineTo(s * 0.15f, s * 0.85f)
        box.lineTo(s * 0.74f, s * 0.85f)
        box.lineTo(s * 0.74f, s * 0.57f)
        drawPath(box, color, style = Stroke(width = sw, cap = StrokeCap.Round, join = StrokeJoin.Round))
        drawLine(color, Offset(s * 0.40f, s * 0.60f), Offset(s * 0.80f, s * 0.20f), sw, StrokeCap.Round)
        drawLine(color, Offset(s * 0.50f, s * 0.20f), Offset(s * 0.80f, s * 0.20f), sw, StrokeCap.Round)
        drawLine(color, Offset(s * 0.80f, s * 0.20f), Offset(s * 0.80f, s * 0.50f), sw, StrokeCap.Round)
    }
}

/** Full-screen page header: back arrow plus a section label. Draws its own hairline. */
@Composable
fun PageHeader(title: String, onBack: () -> Unit, trailing: @Composable (() -> Unit)? = null) {
    val t = LocalTaste.current
    Row(
        Modifier
            .fillMaxWidth()
            .background(t.field)
            .padding(start = 4.dp, end = 12.dp, top = 4.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onClick = onBack) {
            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "back", tint = t.text)
        }
        SectionLabel(title)
        Spacer(Modifier.weight(1f))
        trailing?.invoke()
    }
}

/** Bolt glyph for the inspire tab (no bolt in icons-core). */
@Composable
fun BoltGlyph(tint: Color, modifier: Modifier = Modifier, desc: String? = null) {
    Canvas(
        modifier.semantics { if (desc != null) contentDescription = desc },
    ) {
        val s = size.minDimension
        val p = Path().apply {
            moveTo(s * 0.56f, s * 0.04f)
            lineTo(s * 0.12f, s * 0.58f)
            lineTo(s * 0.48f, s * 0.58f)
            lineTo(s * 0.42f, s * 0.96f)
            lineTo(s * 0.88f, s * 0.40f)
            lineTo(s * 0.52f, s * 0.40f)
            close()
        }
        drawPath(p, tint)
    }
}

/** Shared height of every bottom bar — tab bar, toolbars, detail actions. Keep them equal. */
val BottomBarHeight = 56.dp

/** List-thumb size for the density flip on followups / trash / cleanup. */
fun thumbFor(columns: Int) = when (columns) {
    1 -> 92.dp
    3 -> 44.dp
    4 -> 36.dp
    else -> 64.dp
}
package com.niknair.tasteengine.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import com.niknair.tasteengine.LocalTaste
import com.niknair.tasteengine.Mori
import com.niknair.tasteengine.net.Api
import com.niknair.tasteengine.net.Brief
import com.niknair.tasteengine.vm.VaultVm
import kotlinx.coroutines.launch

/**
 * Inspire: ask for an idea, or press suggest and let the vault answer.
 * One input, one brief. No build-type picker.
 */
@Composable
fun InspireScreen(vm: VaultVm, openDetail: (String) -> Unit) {
    val t = LocalTaste.current
    val focus = LocalFocusManager.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var ask by remember { mutableStateOf(vm.briefAbout) }
    val brief = vm.brief

    Column(
        Modifier
            .fillMaxSize()
            .background(t.field)
            .statusBarsPadding(),
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .padding(start = 16.dp, end = 10.dp, top = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            SectionLabel("inspire")
            Spacer(Modifier.weight(1f))
            if (brief != null || vm.briefError != null) {
                FlatButton(
                    text = "new",
                    ghost = true,
                    onClick = {
                        vm.clearBrief()
                        ask = ""
                    },
                )
            }
        }

        Column(
            Modifier.padding(start = 16.dp, end = 16.dp, top = 10.dp, bottom = 6.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            TasteField(
                value = ask,
                onValueChange = { ask = it },
                placeholder = "an idea, a project — or leave it empty",
                modifier = Modifier.fillMaxWidth(),
                singleLine = false,
                minLines = 2,
            )
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                FlatButton(
                    text = if (vm.briefLoading) "thinking…" else "go",
                    enabled = !vm.briefLoading,
                    onClick = {
                        focus.clearFocus()
                        vm.runBrief(ask.trim())
                    },
                )
                FlatButton(
                    text = "suggest",
                    ghost = true,
                    enabled = !vm.briefLoading,
                    onClick = {
                        focus.clearFocus()
                        ask = ""
                        vm.runBrief("")
                    },
                )
                Spacer(Modifier.weight(1f))
                if (vm.briefLoading) PulseText("reading your vault…", t.accentStrong)
            }
            val err = vm.briefError
            if (err != null) {
                Text(err, color = t.danger, fontSize = 12.sp, fontFamily = Mori)
            }
        }

        BriefBody(
            brief = brief,
            loading = vm.briefLoading,
            openDetail = openDetail,
            onCopyPrompt = {
                scope.launch {
                    val md = vm.briefMarkdown()
                    val body = md.ifBlank { promptOnly(brief) }
                    if (body.isNotBlank()) {
                        val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                        cm.setPrimaryClip(ClipData.newPlainText("build brief", body))
                        Toast.makeText(context, "brief copied", Toast.LENGTH_SHORT).show()
                    }
                }
            },
            modifier = Modifier.weight(1f),
        )
    }
}

private fun promptOnly(brief: Brief?): String = brief?.buildPrompt ?: ""

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun BriefBody(
    brief: Brief?,
    loading: Boolean,
    openDetail: (String) -> Unit,
    onCopyPrompt: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val t = LocalTaste.current
    when {
        brief == null && !loading -> NoteBlock(
            "ask for an idea, or press suggest\n\nthe brief comes from what you already saved",
            modifier = modifier.fillMaxWidth(),
        )
        brief == null -> Box(modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
            PulseText("mixing references…", t.accentStrong, Modifier.padding(top = 24.dp))
        }
        else -> LazyColumn(
            modifier = modifier.fillMaxWidth(),
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 28.dp),
        ) {
            item { BriefHead(brief) }
            if (brief.themes.isNotEmpty()) {
                item { BriefTitle("themes") }
                items(brief.themes.size) { i ->
                    val th = brief.themes[i]
                    BriefCard {
                        Text(th.name, color = t.accentStrong, fontSize = 14.sp, fontFamily = Mori, fontWeight = FontWeight.Medium)
                        if (th.why.isNotEmpty()) {
                            Spacer(Modifier.height(3.dp))
                            Text(th.why, color = t.muted, fontSize = 12.sp, fontFamily = Mori, lineHeight = 17.sp)
                        }
                    }
                }
            }
            if (brief.palette.isNotEmpty()) {
                item { BriefTitle("palette") }
                item { PaletteRow(brief) }
            }
            if (brief.families.isNotEmpty() || brief.scaleNote.isNotEmpty()) {
                item { BriefTitle("type") }
                items(brief.families.size) { i ->
                    val f = brief.families[i]
                    BriefCard {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(f.family, color = t.text, fontSize = 15.sp, fontFamily = Mori, fontWeight = FontWeight.Medium)
                            Spacer(Modifier.width(8.dp))
                            Mark(f.role.ifEmpty { "body" }, color = t.muted)
                            if (f.match.isNotEmpty()) {
                                Spacer(Modifier.width(6.dp))
                                Mark(f.match, color = if (f.match == "observed") t.ok else t.muted)
                            }
                        }
                        if (f.why.isNotEmpty()) {
                            Spacer(Modifier.height(3.dp))
                            Text(f.why, color = t.muted, fontSize = 12.sp, fontFamily = Mori, lineHeight = 17.sp)
                        }
                    }
                }
                if (brief.scaleNote.isNotEmpty()) {
                    item {
                        Text(
                            brief.scaleNote,
                            color = t.muted,
                            fontSize = 12.sp,
                            fontFamily = Mori,
                            lineHeight = 17.sp,
                            modifier = Modifier.padding(top = 4.dp, bottom = 6.dp),
                        )
                    }
                }
            }
            if (brief.shape.isNotEmpty() || brief.parts.isNotEmpty()) {
                item { BriefTitle("build") }
                if (brief.shape.isNotEmpty()) {
                    item {
                        Text(
                            brief.shape,
                            color = t.text,
                            fontSize = 13.sp,
                            fontFamily = Mori,
                            lineHeight = 19.sp,
                            modifier = Modifier.padding(bottom = 8.dp),
                        )
                    }
                }
                items(brief.parts.size) { i ->
                    val p = brief.parts[i]
                    BriefCard {
                        Text(p.name, color = t.accentStrong, fontSize = 13.sp, fontFamily = Mori, fontWeight = FontWeight.Medium)
                        if (p.what.isNotEmpty()) {
                            Spacer(Modifier.height(3.dp))
                            Text(p.what, color = t.muted, fontSize = 12.sp, fontFamily = Mori, lineHeight = 17.sp)
                        }
                    }
                }
            }
            if (brief.make.isNotEmpty()) {
                item { BriefTitle("make") }
                items(brief.make.size) { i ->
                    val s = brief.make[i]
                    Row(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                        Text(
                            "${i + 1}",
                            color = t.accentStrong,
                            fontSize = 13.sp,
                            fontFamily = Mori,
                            fontWeight = FontWeight.Medium,
                            modifier = Modifier.width(22.dp),
                        )
                        Column {
                            Text(s.step, color = t.text, fontSize = 13.sp, fontFamily = Mori)
                            if (s.detail.isNotEmpty()) {
                                Text(s.detail, color = t.muted, fontSize = 12.sp, fontFamily = Mori, lineHeight = 17.sp)
                            }
                        }
                    }
                }
            }
            if (brief.steal.isNotEmpty()) {
                item { BriefTitle("steal") }
                items(brief.steal.size) { i ->
                    Row(Modifier.fillMaxWidth().padding(vertical = 3.dp)) {
                        Text("·", color = t.accentStrong, fontSize = 13.sp, modifier = Modifier.width(12.dp))
                        Text(brief.steal[i], color = t.text, fontSize = 13.sp, fontFamily = Mori, lineHeight = 18.sp)
                    }
                }
            }
            if (brief.buildPrompt.isNotEmpty()) {
                item { BriefTitle("build prompt") }
                item {
                    Column(
                        Modifier
                            .fillMaxWidth()
                            .background(t.surface2)
                            .border(1.dp, t.line, RoundedCornerShape(0.dp))
                            .padding(12.dp),
                    ) {
                        Text(
                            brief.buildPrompt,
                            color = t.text,
                            fontSize = 12.sp,
                            fontFamily = Mori,
                            lineHeight = 18.sp,
                        )
                        Spacer(Modifier.height(8.dp))
                        FlatButton(text = "copy", ghost = true, onClick = onCopyPrompt)
                    }
                }
            }
            if (brief.refs.isNotEmpty()) {
                item { BriefTitle("from your vault") }
                item {
                    // horizontal strip: no empty gutter on the right, and the
                    // wall scrolls instead of wrapping into a ragged grid.
                    LazyRow(
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                        contentPadding = PaddingValues(end = 16.dp),
                    ) {
                        items(brief.refs.size) { i ->
                            val r = brief.refs[i]
                            Box(
                                Modifier
                                    .width(96.dp)
                                    .height(120.dp)
                                    .background(t.surface)
                                    .border(1.dp, t.line, RoundedCornerShape(0.dp))
                                    .clickable { openDetail(r.id) },
                            ) {
                                if (r.thumb.isNotEmpty()) {
                                    AsyncImage(
                                        model = Api.mediaUrl(r.thumb),
                                        contentDescription = r.title,
                                        contentScale = ContentScale.Crop,
                                        modifier = Modifier.fillMaxSize(),
                                    )
                                } else {
                                    Text(
                                        r.title.take(60),
                                        color = t.muted,
                                        fontSize = 10.sp,
                                        fontFamily = Mori,
                                        modifier = Modifier.padding(6.dp),
                                    )
                                }
                                Box(
                                    Modifier
                                        .align(Alignment.TopStart)
                                        .background(Color(0xB3120A0D))
                                        .padding(horizontal = 4.dp, vertical = 2.dp),
                                ) {
                                    Text(
                                        r.ref.ifEmpty { "r$i" },
                                        color = Color(0xFFFDF6F6),
                                        fontSize = 9.sp,
                                        fontFamily = Mori,
                                    )
                                }
                            }
                        }
                    }
                }
            }
            if (brief.lensName.isNotEmpty()) {
                item {
                    Text(
                        "open brainstorm · lens: ${brief.lensName}",
                        color = t.muted,
                        fontSize = 11.sp,
                        fontFamily = Mori,
                        modifier = Modifier.padding(top = 12.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun BriefHead(brief: Brief) {
    val t = LocalTaste.current
    Column(Modifier.fillMaxWidth().padding(top = 4.dp, bottom = 4.dp)) {
        if (brief.kind.isNotEmpty()) {
            Mark(brief.kind, color = t.accentStrong)
        }
        Spacer(Modifier.height(6.dp))
        Text(
            brief.title.ifEmpty { "build brief" },
            color = t.text,
            fontSize = 21.sp,
            fontFamily = Mori,
            fontWeight = FontWeight.Medium,
            lineHeight = 26.sp,
        )
        if (brief.oneLiner.isNotEmpty()) {
            Spacer(Modifier.height(6.dp))
            Text(
                brief.oneLiner,
                color = t.muted,
                fontSize = 13.sp,
                fontFamily = Mori,
                lineHeight = 18.sp,
            )
        }
    }
}

@Composable
private fun BriefTitle(text: String) {
    SectionLabel(text, Modifier.padding(top = 16.dp, bottom = 6.dp))
}

@Composable
private fun BriefCard(content: @Composable () -> Unit) {
    val t = LocalTaste.current
    Column(
        Modifier
            .fillMaxWidth()
            .padding(bottom = 6.dp)
            .background(t.surface2)
            .border(1.dp, t.lineSoft, RoundedCornerShape(0.dp))
            .padding(10.dp),
    ) {
        content()
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun PaletteRow(brief: Brief) {
    val t = LocalTaste.current
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        brief.palette.forEach { sw ->
            val c = hexColor(sw.hex)
            Column(Modifier.width(78.dp)) {
                Box(
                    Modifier
                        .fillMaxWidth()
                        .height(46.dp)
                        .background(c ?: t.surface2)
                        .border(1.dp, t.line, RoundedCornerShape(0.dp)),
                )
                Spacer(Modifier.height(3.dp))
                Text(
                    sw.hex.lowercase(),
                    color = t.text,
                    fontSize = 10.sp,
                    fontFamily = Mori,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    sw.role.ifEmpty { sw.name },
                    color = t.muted,
                    fontSize = 9.sp,
                    fontFamily = Mori,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

private fun hexColor(hex: String): Color? = runCatching {
    Color(android.graphics.Color.parseColor(hex.trim()))
}.getOrNull()

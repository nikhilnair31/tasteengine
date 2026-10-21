package com.niknair.tasteengine.ui

import android.content.Context
import android.content.Intent
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import com.niknair.tasteengine.LocalTaste
import com.niknair.tasteengine.Mori
import com.niknair.tasteengine.net.Api
import com.niknair.tasteengine.net.Followup
import com.niknair.tasteengine.net.ItemCard
import com.niknair.tasteengine.net.ItemDetail
import com.niknair.tasteengine.net.MediaInfo
import com.niknair.tasteengine.net.Parse
import com.niknair.tasteengine.vm.VaultVm
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DetailScreen(vm: VaultVm, itemId: String, onBack: () -> Unit, openDetail: (String) -> Unit) {
    val t = LocalTaste.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var detail by remember { mutableStateOf<ItemDetail?>(null) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    var showDelete by remember { mutableStateOf(false) }
    var viewerIndex by remember { mutableStateOf<Int?>(null) }

    LaunchedEffect(itemId) {
        loading = true
        error = null
        viewerIndex = null
        try {
            detail = Parse.detail(Api.item(itemId))
            Api.interact(itemId)
        } catch (e: Exception) {
            error = e.message ?: "could not load"
        }
        loading = false
    }

    val reload: () -> Unit = {
        scope.launch {
            runCatching { detail = Parse.detail(Api.item(itemId)) }
        }
    }

    BackHandler(enabled = viewerIndex != null) { viewerIndex = null }

    Box(Modifier.fillMaxSize()) {
        Column(
            Modifier
                .fillMaxSize()
                .background(t.canvas)
                .statusBarsPadding(),
        ) {
            // -------------------------------------------------------- topbar
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 4.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconButton(onClick = onBack) {
                    Icon(Icons.Default.ArrowBack, contentDescription = "back", tint = t.text)
                }
            }

            when {
                loading -> DetailSkeleton(stub = vm.detailStub)
                detail == null -> NoteBlock(error ?: "not found")
                else -> {
                    val d = detail!!
                    Column(
                        Modifier
                            .weight(1f)
                            .fillMaxWidth()
                            .verticalScroll(rememberScrollState())
                            .padding(bottom = 24.dp),
                    ) {
                        val hero = d.media.firstOrNull { it.kind == "image" } ?: d.media.firstOrNull()
                        if (hero != null && hero.best.isNotEmpty()) {
                            Box(
                                Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 12.dp)
                                    .background(t.surface)
                                    .border(1.dp, t.line, RoundedCornerShape(0.dp))
                                    .clickable {
                                        if (hero.isVideo) {
                                            openUrl(context, d.card.url)
                                        } else {
                                            val idx = d.media.indexOfFirst { it.id == hero.id }
                                            viewerIndex = if (idx >= 0) idx else 0
                                        }
                                    },
                            ) {
                                AsyncImage(
                                    model = Api.mediaUrl(hero.file.ifEmpty { hero.best }),
                                    contentDescription = d.card.title,
                                    contentScale = ContentScale.Fit,
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .aspectRatio(1f / hero.aspect),
                                )
                                if (hero.isVideo) {
                                    Box(
                                        Modifier
                                            .align(Alignment.Center)
                                            .size(54.dp)
                                            .background(Color(0xB3120A0D)),
                                        contentAlignment = Alignment.Center,
                                    ) {
                                        Icon(
                                            Icons.Default.PlayArrow,
                                            contentDescription = "play",
                                            tint = Color(0xFFFDF6F6),
                                            modifier = Modifier.size(30.dp),
                                        )
                                    }
                                }
                            }
                        }

                        if (d.media.size > 1) {
                            LazyRow(
                                Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 12.dp, vertical = 8.dp),
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                itemsIndexed(d.media.drop(1)) { i, m ->
                                    Box(
                                        Modifier
                                            .size(78.dp)
                                            .background(t.surface2)
                                            .border(1.dp, t.line, RoundedCornerShape(0.dp))
                                            .clickable {
                                                if (m.isVideo) {
                                                    openUrl(context, d.card.url)
                                                } else {
                                                    viewerIndex = i + 1
                                                }
                                            },
                                    ) {
                                        if (m.best.isNotEmpty()) {
                                            AsyncImage(
                                                model = Api.mediaUrl(m.best),
                                                contentDescription = null,
                                                contentScale = ContentScale.Crop,
                                                modifier = Modifier.fillMaxSize(),
                                            )
                                        }
                                        if (m.isVideo) {
                                            Icon(
                                                Icons.Default.PlayArrow,
                                                contentDescription = null,
                                                tint = Color(0xFFFDF6F6),
                                                modifier = Modifier
                                                    .align(Alignment.Center)
                                                    .size(22.dp),
                                            )
                                        }
                                    }
                                }
                            }
                        }

                        Column(Modifier.padding(horizontal = 16.dp)) {
                            Spacer(Modifier.height(12.dp))
                            val title = d.card.title.ifEmpty { d.card.text.take(140).trim() }.ifEmpty { d.card.url }
                            if (title.isNotEmpty()) {
                                Text(
                                    text = title,
                                    color = t.text,
                                    fontSize = 19.sp,
                                    fontFamily = Mori,
                                    fontWeight = FontWeight.SemiBold,
                                    lineHeight = 25.sp,
                                )
                                Spacer(Modifier.height(10.dp))
                            }
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Mark(d.card.source.ifEmpty { "vault" })
                                Spacer(Modifier.width(10.dp))
                                Mark(d.card.kind, color = t.muted)
                                if (d.card.savedAt.isNotEmpty()) {
                                    Spacer(Modifier.width(10.dp))
                                    Text(
                                        d.card.savedAt.take(10),
                                        color = t.muted,
                                        fontSize = 11.sp,
                                        fontFamily = Mori,
                                    )
                                }
                                if (d.card.author.isNotEmpty()) {
                                    Spacer(Modifier.width(10.dp))
                                    Text(
                                        d.card.author.take(46),
                                        color = t.muted,
                                        fontSize = 11.sp,
                                        fontFamily = Mori,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                }
                            }
                            if (d.card.tags.isNotEmpty()) {
                                Spacer(Modifier.height(16.dp))
                                LazyRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                                    items(d.card.tags) { tag -> SmallTag(tag) }
                                }
                            }
                            if (d.card.note.isNotBlank()) {
                                Spacer(Modifier.height(18.dp))
                                SectionLabel("note")
                                Spacer(Modifier.height(8.dp))
                                Panel {
                                    Text(
                                        d.card.note,
                                        color = t.text,
                                        fontSize = 13.sp,
                                        fontFamily = Mori,
                                        lineHeight = 19.sp,
                                    )
                                }
                            }

                            // -------------------------------------------- explainer
                            Spacer(Modifier.height(18.dp))
                            SectionLabel("explainer")
                            Spacer(Modifier.height(8.dp))
                            val ex = d.explanation
                            Panel {
                                Column {
                                    when {
                                        ex != null -> ExplainBlock(ex)
                                        d.card.jobState == "queued" || d.card.jobState == "working" -> Text(
                                            "still pulling the post…",
                                            color = t.muted,
                                            fontSize = 13.sp,
                                            fontFamily = Mori,
                                        )
                                        d.card.text.isBlank() -> Text(
                                            "explainer not generated.",
                                            color = t.muted,
                                            fontSize = 13.sp,
                                            fontFamily = Mori,
                                        )
                                    }
                                    if (d.card.text.isNotBlank()) {
                                        Spacer(Modifier.height(14.dp))
                                        SectionLabel(if (d.card.source == "mymind") "saved text" else "text")
                                        Spacer(Modifier.height(5.dp))
                                        Text(
                                            d.card.text,
                                            color = t.text,
                                            fontSize = 13.sp,
                                            fontFamily = Mori,
                                            lineHeight = 19.sp,
                                        )
                                    }
                                }
                            }

                            // -------------------------------------------- followups
                            if (d.followups.isNotEmpty()) {
                                Spacer(Modifier.height(20.dp))
                                SectionLabel("follow-ups")
                                Spacer(Modifier.height(8.dp))
                                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                    d.followups.forEach { f ->
                                        FollowupRow(f) { openUrl(context, f.url) }
                                    }
                                }
                            }

                            // -------------------------------------------- outlinks
                            if (d.outlinks.isNotEmpty()) {
                                Spacer(Modifier.height(20.dp))
                                SectionLabel("links")
                                Spacer(Modifier.height(8.dp))
                                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                    d.outlinks.forEach { (label, url) ->
                                        LinkRow(label, url) { openUrl(context, url) }
                                    }
                                }
                            }

                            Spacer(Modifier.height(20.dp))
                        }

                        // ------------------------------------------------ similar
                        if (d.similar.isNotEmpty()) {
                            Column(Modifier.padding(horizontal = 16.dp)) {
                                SectionLabel("similar")
                                Spacer(Modifier.height(8.dp))
                                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                    d.similar.chunked(2).forEach { pair ->
                                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                            pair.forEach { s ->
                                                MiniCard(s, Modifier.weight(1f)) { openDetail(s.id) }
                                            }
                                            if (pair.size == 1) Spacer(Modifier.weight(1f))
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            val dBar = detail
            if (!loading && dBar != null) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .background(t.canvas)
                        .navigationBarsPadding()
                        .drawBehind {
                            drawLine(
                                color = t.line,
                                start = Offset(0f, 0f),
                                end = Offset(size.width, 0f),
                                strokeWidth = 1.dp.toPx(),
                            )
                        },
                ) {
                    DetailAction(onClick = {
                        vm.refreshItem(itemId)
                        scope.launch {
                            delay(5000)
                            reload()
                        }
                    }) {
                        Icon(
                            Icons.Default.Refresh,
                            contentDescription = "refresh",
                            tint = t.text,
                            modifier = Modifier.size(23.dp),
                        )
                    }
                    if (dBar.card.url.isNotEmpty()) {
                        DetailAction(onClick = { openUrl(context, dBar.card.url) }) {
                            Box(Modifier.semantics { contentDescription = "source" }) {
                                ExternalGlyph(t.text, Modifier.size(23.dp))
                            }
                        }
                    }
                    DetailAction(onClick = { shareDetail(context, dBar) }) {
                        Icon(
                            Icons.Default.Share,
                            contentDescription = "share",
                            tint = t.text,
                            modifier = Modifier.size(23.dp),
                        )
                    }
                    DetailAction(onClick = { showDelete = true }) {
                        Icon(
                            Icons.Default.Delete,
                            contentDescription = "delete",
                            tint = t.danger,
                            modifier = Modifier.size(23.dp),
                        )
                    }
                }
            }
        }

        // full-screen media viewer (view-only)
        val d = detail
        if (viewerIndex != null && d != null && d.media.isNotEmpty()) {
            GalleryOverlay(
                media = d.media,
                initialIndex = (viewerIndex ?: 0).coerceIn(0, d.media.size - 1),
                onClose = { viewerIndex = null },
                onOpenVideo = { openUrl(context, d.card.url) },
            )
        }
    }

    if (showDelete) {
        AlertDialog(
            onDismissRequest = { showDelete = false },
            containerColor = t.surface,
            title = {
                Text("delete post?", color = t.text, fontFamily = Mori, fontSize = 16.sp)
            },
            text = {
                Text("moves to trash", color = t.muted, fontFamily = Mori, fontSize = 13.sp)
            },
            confirmButton = {
                FlatButton(
                    text = "delete",
                    danger = true,
                    onClick = {
                        showDelete = false
                        vm.deleteItem(itemId) { onBack() }
                    },
                )
            },
            dismissButton = {
                FlatButton(text = "cancel", ghost = true, onClick = { showDelete = false })
            },
        )
    }
}

/** Bottom action slot: icon only, centered in the shared bar height. */
/** Shimmer skeleton while the detail loads; paints the tapped card's cover instantly when cached. */
@Composable
private fun DetailSkeleton(stub: ItemCard?) {
    val t = LocalTaste.current
    Column(
        Modifier
            .fillMaxSize()
            .padding(horizontal = 12.dp, vertical = 6.dp),
    ) {
        val cover = stub?.cover
        val src = cover?.let { it.file.ifEmpty { it.best } }.orEmpty()
        Box(
            Modifier
                .fillMaxWidth()
                .background(t.surface)
                .border(1.dp, t.line, RoundedCornerShape(0.dp)),
        ) {
            if (src.isNotEmpty()) {
                val ar = cover?.aspect?.takeIf { it > 0f } ?: 1f
                AsyncImage(
                    model = Api.mediaUrl(src),
                    contentDescription = null,
                    contentScale = ContentScale.Fit,
                    modifier = Modifier
                        .fillMaxWidth()
                        .aspectRatio(1f / ar),
                )
            } else {
                ShimmerBox(
                    Modifier
                        .fillMaxWidth()
                        .aspectRatio(1.1f),
                )
            }
        }
        Spacer(Modifier.height(16.dp))
        ShimmerBox(
            Modifier
                .fillMaxWidth(0.8f)
                .height(18.dp),
        )
        Spacer(Modifier.height(10.dp))
        ShimmerBox(
            Modifier
                .fillMaxWidth(0.55f)
                .height(12.dp),
        )
        Spacer(Modifier.height(22.dp))
        repeat(3) {
            ShimmerBox(
                Modifier
                    .fillMaxWidth()
                    .height(64.dp),
            )
            Spacer(Modifier.height(10.dp))
        }
    }
}

@Composable
private fun RowScope.DetailAction(
    onClick: () -> Unit,
    content: @Composable () -> Unit,
) {
    Column(
        Modifier
            .weight(1f)
            .height(BottomBarHeight)
            .clickable { onClick() },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        content()
    }
}

private fun shareDetail(context: Context, d: ItemDetail) {
    val send = Intent(Intent.ACTION_SEND).apply {
        type = "text/plain"
        putExtra(Intent.EXTRA_TEXT, d.card.url.ifEmpty { d.card.title })
        putExtra(Intent.EXTRA_SUBJECT, d.card.title)
    }
    context.startActivity(Intent.createChooser(send, "share"))
}

/** Full-screen viewer: swipe through every media item, tap image to close. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun GalleryOverlay(
    media: List<MediaInfo>,
    initialIndex: Int,
    onClose: () -> Unit,
    onOpenVideo: () -> Unit,
) {
    val t = LocalTaste.current
    val pagerState = rememberPagerState(initialPage = initialIndex) { media.size }
    Box(
        Modifier
            .fillMaxSize()
            .background(Color(0xFF0B0709))
            .navigationBarsPadding(),
    ) {
        HorizontalPager(
            state = pagerState,
            modifier = Modifier.fillMaxSize(),
            verticalAlignment = Alignment.CenterVertically,
        ) { page ->
            val m = media[page]
            Box(
                Modifier
                    .fillMaxSize()
                    .clickable { if (m.isVideo) onOpenVideo() else onClose() },
                contentAlignment = Alignment.Center,
            ) {
                if (m.isVideo) {
                    AsyncImage(
                        model = Api.mediaUrl(m.best),
                        contentDescription = null,
                        contentScale = ContentScale.Fit,
                        modifier = Modifier.fillMaxSize(),
                    )
                    Box(
                        Modifier
                            .size(64.dp)
                            .background(Color(0xB3120A0D)),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            Icons.Default.PlayArrow,
                            contentDescription = "play",
                            tint = Color(0xFFFDF6F6),
                            modifier = Modifier.size(36.dp),
                        )
                    }
                } else {
                    AsyncImage(
                        model = Api.mediaUrl(m.file.ifEmpty { m.best }),
                        contentDescription = null,
                        contentScale = ContentScale.Fit,
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        IconButton(
            onClick = onClose,
            modifier = Modifier
                .align(Alignment.TopEnd)
                .statusBarsPadding()
                .padding(10.dp),
        ) {
            Icon(Icons.Default.Close, contentDescription = "close", tint = Color(0xFFFDF6F6))
        }
        if (media.size > 1) {
            Text(
                text = "${pagerState.currentPage + 1} / ${media.size}",
                color = Color(0xFFFDF6F6),
                fontSize = 12.sp,
                fontFamily = Mori,
                modifier = Modifier
                    .align(Alignment.BottomCenter)
                    .padding(bottom = 28.dp)
                    .background(Color(0x8C120A0D), RoundedCornerShape(0.dp))
                    .padding(horizontal = 12.dp, vertical = 5.dp),
            )
        }
    }
}

@Composable
private fun Panel(content: @Composable () -> Unit) {
    val t = LocalTaste.current
    Box(
        Modifier
            .fillMaxWidth()
            .background(t.surface2)
            .border(1.dp, t.lineSoft, RoundedCornerShape(0.dp))
            .padding(14.dp),
    ) {
        content()
    }
}

@Composable
private fun SmallTag(text: String) {
    val t = LocalTaste.current
    Box(
        Modifier
            .background(t.surface2, RoundedCornerShape(0.dp))
            .border(1.dp, t.lineSoft, RoundedCornerShape(0.dp))
            .padding(horizontal = 10.dp, vertical = 4.dp),
    ) {
        Text(text, color = t.muted, fontSize = 11.sp, fontFamily = Mori, maxLines = 1)
    }
}

@Composable
private fun FollowupRow(f: Followup, onOpen: () -> Unit) {
    val t = LocalTaste.current
    Column(
        Modifier
            .fillMaxWidth()
            .background(t.surface2)
            .border(1.dp, t.lineSoft, RoundedCornerShape(0.dp))
            .clickable { onOpen() }
            .padding(11.dp),
    ) {
        Mark(f.kind.ifEmpty { "link" })
        Spacer(Modifier.height(5.dp))
        Text(
            text = f.title.ifEmpty { f.url },
            color = t.text,
            fontSize = 13.sp,
            fontFamily = Mori,
            lineHeight = 18.sp,
            maxLines = 3,
            overflow = TextOverflow.Ellipsis,
        )
        if (f.why.isNotEmpty()) {
            Spacer(Modifier.height(3.dp))
            Text(f.why, color = t.muted, fontSize = 11.sp, fontFamily = Mori, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
private fun LinkRow(label: String, url: String, onOpen: () -> Unit) {
    val t = LocalTaste.current
    Row(
        Modifier
            .fillMaxWidth()
            .background(t.surface2)
            .border(1.dp, t.lineSoft, RoundedCornerShape(0.dp))
            .clickable { onOpen() }
            .padding(11.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                label,
                color = t.text,
                fontSize = 13.sp,
                fontFamily = Mori,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Spacer(Modifier.height(2.dp))
            Text(
                url.removePrefix("https://").removePrefix("http://").take(60),
                color = t.muted,
                fontSize = 11.sp,
                fontFamily = Mori,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Spacer(Modifier.width(8.dp))
        Mark("open", color = t.accentStrong)
    }
}

@Composable
private fun MiniCard(item: ItemCard, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val t = LocalTaste.current
    val cover = item.cover
    Column(
        modifier
            .background(t.surface)
            .border(1.dp, t.line, RoundedCornerShape(0.dp))
            .clickable { onClick() },
    ) {
        Box(
            Modifier
                .fillMaxWidth()
                .aspectRatio(1f)
                .background(t.surface2),
        ) {
            if (cover != null && cover.best.isNotEmpty()) {
                AsyncImage(
                    model = Api.mediaUrl(cover.best),
                    contentDescription = item.title,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.fillMaxSize(),
                )
            }
        }
        Column(Modifier.padding(7.dp)) {
            Text(
                item.title.ifEmpty { item.text.take(60) },
                color = t.text,
                fontSize = 11.sp,
                fontFamily = Mori,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

// ------------------------------------------------------------ explainer

@Composable
private fun ExplainBlock(ex: JSONObject) {
    val order = listOf(
        "summary", "visuals", "mood", "style_dna", "why_it_fits",
        "color_words", "video_note", "sections",
    )
    val keys = mutableListOf<String>()
    for (k in order) {
        if (ex.has(k) && !ex.isNull(k)) keys.add(k)
    }
    val it = ex.keys()
    while (it.hasNext()) {
        val k = it.next()
        if (k != "tags" && !keys.contains(k) && !ex.isNull(k)) keys.add(k)
    }
    Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
        for (k in keys) {
            val v = ex.opt(k)
            if (v == null || v == JSONObject.NULL) continue
            Column {
                SectionLabel(k.replace('_', ' '))
                Spacer(Modifier.height(5.dp))
                ExplainValue(v)
            }
        }
    }
}

@Composable
private fun ExplainValue(v: Any) {
    val t = LocalTaste.current
    when (v) {
        is String -> if (v.isNotBlank()) {
            Text(v, color = t.text, fontSize = 13.sp, fontFamily = Mori, lineHeight = 19.sp)
        }
        is JSONArray -> {
            val list = mutableListOf<Any>()
            for (i in 0 until v.length()) {
                val child = v.opt(i)
                if (child != null && child != JSONObject.NULL) list.add(child)
            }
            if (list.all { it is String }) {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    list.forEach { s -> BulletRow(s as String) }
                }
            } else {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    list.forEach { child ->
                        if (child is JSONObject) {
                            Column {
                                val ttl = child.optString("title")
                                if (ttl.isNotEmpty()) {
                                    Text(
                                        ttl,
                                        color = t.text,
                                        fontSize = 13.sp,
                                        fontFamily = Mori,
                                        fontWeight = FontWeight.SemiBold,
                                    )
                                }
                                val body = child.optString("body").ifEmpty { child.optString("text") }
                                if (body.isNotEmpty()) {
                                    Spacer(Modifier.height(3.dp))
                                    Text(
                                        body,
                                        color = t.text,
                                        fontSize = 13.sp,
                                        fontFamily = Mori,
                                        lineHeight = 19.sp,
                                    )
                                }
                            }
                        } else if (child is String) {
                            BulletRow(child)
                        }
                    }
                }
            }
        }
        is JSONObject -> {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                val it = v.keys()
                while (it.hasNext()) {
                    val k = it.next()
                    val child = v.opt(k)
                    if (child == null || child == JSONObject.NULL) continue
                    Column {
                        SectionLabel(k.replace('_', ' '))
                        Spacer(Modifier.height(4.dp))
                        ExplainValue(child)
                    }
                }
            }
        }
        else -> Text(v.toString(), color = t.text, fontSize = 13.sp, fontFamily = Mori)
    }
}

@Composable
private fun BulletRow(text: String) {
    val t = LocalTaste.current
    Row {
        Text("–", color = t.accentStrong, fontSize = 13.sp, fontFamily = Mori)
        Spacer(Modifier.width(8.dp))
        Text(text, color = t.text, fontSize = 13.sp, fontFamily = Mori, lineHeight = 19.sp)
    }
}
package com.niknair.tasteengine.ui

import android.net.Uri
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.lazy.staggeredgrid.LazyVerticalStaggeredGrid
import androidx.compose.foundation.lazy.staggeredgrid.StaggeredGridCells
import androidx.compose.foundation.lazy.staggeredgrid.StaggeredGridItemSpan
import androidx.compose.foundation.lazy.staggeredgrid.items as staggeredItems
import androidx.compose.foundation.lazy.staggeredgrid.rememberLazyStaggeredGridState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import com.niknair.tasteengine.LocalTaste
import com.niknair.tasteengine.AppFont
import com.niknair.tasteengine.net.Api
import com.niknair.tasteengine.net.ItemCard
import com.niknair.tasteengine.vm.VaultVm

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun GridScreen(vm: VaultVm, openDetail: (String) -> Unit) {
    val t = LocalTaste.current
    val context = LocalContext.current
    val focus = LocalFocusManager.current
    val gridState = rememberLazyStaggeredGridState()
    var query by remember { mutableStateOf(vm.grid.q) }
    var showNote by remember { mutableStateOf(false) }
    var showSheet by remember { mutableStateOf(false) }
    var searchFocused by remember { mutableStateOf(false) }

    // load-more watch: read scroll state inside snapshotFlow, NOT in composition —
    // reading layoutInfo during composition recomposed the whole screen on every
    // scroll frame (the fast-scroll stutter).
    LaunchedEffect(gridState) {
        snapshotFlow {
            val info = gridState.layoutInfo
            (info.visibleItemsInfo.lastOrNull()?.index ?: -1) to info.totalItemsCount
        }
            .distinctUntilChanged()
            .collect { (lastVisible, _) ->
                if (vm.grid.items.isNotEmpty() &&
                    lastVisible >= vm.grid.items.size - 4 &&
                    vm.grid.hasMore &&
                    !vm.grid.loading
                ) {
                    vm.loadMore()
                }
            }
    }

    Column(
        Modifier
            .fillMaxSize()
            .background(t.field)
            .statusBarsPadding(),
    ) {
        // ------------------------------------------------------------ grid
        PullToRefreshBox(
            isRefreshing = vm.grid.refreshing,
            onRefresh = { vm.reload() },
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth(),
        ) {
            val g = vm.grid
            when {
                g.items.isEmpty() && g.loading -> ShimmerGrid(columns = vm.gridColumns)
                g.items.isEmpty() && g.error != null -> NoteBlock(g.error ?: "error", retry = { vm.reload() })
                g.items.isEmpty() -> NoteBlock("nothing here yet — save something")
                else -> LazyVerticalStaggeredGrid(
                    columns = StaggeredGridCells.Fixed(vm.gridColumns),
                    state = gridState,
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(start = 12.dp, end = 12.dp, top = 8.dp, bottom = 100.dp),
                    verticalItemSpacing = 10.dp,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    staggeredItems(
                        g.items,
                        key = { it.id },
                        contentType = { item ->
                            if (item.cover != null && item.cover.best.isNotEmpty()) "cover" else "text"
                        },
                    ) { item ->
                        VaultCard(item) {
                            vm.interact(item.id)
                            openDetail(item.id)
                        }
                    }
                    if (g.loading && g.items.isNotEmpty()) {
                        item(span = StaggeredGridItemSpan.FullLine) {
                            Row(
                                Modifier
                                    .fillMaxWidth()
                                    .padding(3.dp),
                                horizontalArrangement = Arrangement.spacedBy(3.dp),
                            ) {
                                repeat(3) {
                                    ShimmerBox(
                                        Modifier
                                            .weight(1f)
                                            .height(120.dp),
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }

        // ------------------------------------------------------------ bottom bar
        Row(
            Modifier
                .fillMaxWidth()
                .height(BottomBarHeight)
                .background(t.surface)
                .padding(start = 10.dp, end = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            SearchRow(
                value = query,
                onValueChange = { query = it },
                placeholder = "search the vault",
                focused = searchFocused,
                onFocusChange = { searchFocused = it },
                onSubmit = {
                    focus.clearFocus()
                    vm.submitSearch(query)
                },
                onClear = {
                    query = ""
                    focus.clearFocus()
                    vm.clearSearch()
                },
                modifier = Modifier
                    .weight(1f)
                    .height(44.dp),
            )
            Spacer(Modifier.width(6.dp))
            ToolIcon(active = showSheet || vm.activeFilterCount() > 0, onClick = { showSheet = true }, desc = "filters and view") {
                GridGlyph(vm.gridColumns, if (showSheet || vm.activeFilterCount() > 0) t.accentStrong else t.text)
            }
        }
    }

    if (showNote) {
        QuickNoteSheet(
            vm = vm,
            onDismiss = { showNote = false },
            onSaved = { message ->
                showNote = false
                Toast.makeText(context, message, Toast.LENGTH_SHORT).show()
            },
        )
    }
    if (showSheet) {
        ViewSheet(vm = vm, onDismiss = { showSheet = false }, onNewSave = { showNote = true })
    }
}

private val sortOptions = listOf(
    "random" to "shuffle",
    "recent" to "newest",
    "oldest" to "oldest",
    "alpha_asc" to "a → z",
    "alpha_desc" to "z → a",
    "frequent" to "most opened",
)

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
private fun ViewSheet(vm: VaultVm, onDismiss: () -> Unit, onNewSave: () -> Unit) {
    val t = LocalTaste.current
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = t.surface,
        dragHandle = null,
    ) {
        Column(
            Modifier
                .fillMaxWidth()
                .heightIn(max = 620.dp)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 18.dp)
                .padding(top = 18.dp)
                .padding(bottom = 26.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                SectionLabel("view")
                Spacer(Modifier.weight(1f))
                FlatButton("new save", onClick = {
                    onDismiss()
                    onNewSave()
                })
            }

            Spacer(Modifier.height(14.dp))
            SectionLabel("grid")
            Spacer(Modifier.height(6.dp))
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                listOf(2, 3, 4, 1).forEach { n ->
                    Chip("$n col", active = vm.gridColumns == n, count = null) { vm.changeGridColumns(n) }
                }
            }

            Spacer(Modifier.height(16.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                SectionLabel("sort")
                Spacer(Modifier.weight(1f))
                if (vm.activeFilterCount() > 0) {
                    FlatButton("clear all", ghost = true, onClick = { vm.clearFilters() })
                }
            }
            Spacer(Modifier.height(6.dp))
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                sortOptions.forEach { (value, label) ->
                    Chip(label, active = vm.grid.sort == value, count = null) { vm.setSort(value) }
                }
            }

            Spacer(Modifier.height(16.dp))
            FilterGroup("kind", vm.kinds, vm.grid.kind) { vm.setKind(it) }
            FilterGroup("source", vm.sources, vm.grid.source) { vm.setSource(it) }
            FilterGroup("hue", vm.hues, vm.grid.hue) { vm.setHue(it) }
            FilterGroup("tag", vm.tags, vm.grid.tag) { vm.setTag(it) }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun FilterGroup(
    label: String,
    options: List<Pair<String, Int>>,
    current: String,
    onPick: (String) -> Unit,
) {
    if (options.isEmpty()) return
    SectionLabel(label)
    Spacer(Modifier.height(6.dp))
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        options.forEach { (name, count) ->
            Chip(name, name == current, count) { onPick(if (name == current) "" else name) }
        }
    }
    Spacer(Modifier.height(14.dp))
}

/** Card mirrors the website: the media (or the post text) IS the card — no title strip. */
@Composable
fun VaultCard(item: ItemCard, onClick: () -> Unit) {
    val t = LocalTaste.current
    val cover = item.cover
    val ratio = if (cover != null) 1f / cover.aspect else 1f
    val processing = item.jobState != null && item.jobState != "done" && item.jobState != "error"
    val bodyText = item.text.trim()

    Box(
        Modifier
            .fillMaxWidth()
            .background(t.surface)
            .border(1.dp, t.line, RoundedCornerShape(0.dp))
            .clickable { onClick() },
    ) {
        Box(
            Modifier
                .fillMaxWidth()
                .aspectRatio(ratio)
                .background(t.surface2),
        ) {
            when {
                cover != null && cover.best.isNotEmpty() -> {
                val coverUrl = cover.thumb.ifBlank { cover.best }
                val appCtx = LocalContext.current
                val coverReq = remember(coverUrl) {
                    coil.request.ImageRequest.Builder(appCtx)
                        .data(Api.mediaUrl(coverUrl))
                        .size(700)
                        .build()
                }
                AsyncImage(
                    model = coverReq,
                    contentDescription = item.title,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.fillMaxSize(),
                )
                    if (cover.isVideo) {
                        Box(
                            Modifier
                                .align(Alignment.BottomStart)
                                .background(Color(0xB3120A0D))
                                .padding(horizontal = 6.dp, vertical = 3.dp),
                        ) {
                            Mark("video", color = Color(0xFFFDF6F6))
                        }
                    }
                }
                bodyText.isNotEmpty() -> {
                    Column(
                        Modifier
                            .fillMaxSize()
                            .padding(9.dp),
                    ) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Mark(item.source.ifEmpty { item.kind }, color = t.accentStrong)
                            if (processing) {
                                Spacer(Modifier.width(8.dp))
                                PulseText("pulling", t.accentStrong)
                            }
                        }
                        Spacer(Modifier.height(6.dp))
                        Text(
                            bodyText.take(260).trim(),
                            color = t.text,
                            fontSize = 12.sp,
                            fontFamily = AppFont,
                            lineHeight = 16.sp,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                processing -> {
                    ShimmerBox(Modifier.fillMaxSize())
                    PulseText(
                        "pulling media",
                        t.accentStrong,
                        Modifier.align(Alignment.Center),
                    )
                }
                else -> {
                    Text(
                        text = item.kind.ifEmpty { "saved" }.uppercase(),
                        color = t.muted,
                        fontSize = 11.sp,
                        fontFamily = AppFont,
                        letterSpacing = 1.5.sp,
                        modifier = Modifier.align(Alignment.Center),
                    )
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun QuickNoteSheet(vm: VaultVm, onDismiss: () -> Unit, onSaved: (String) -> Unit) {
    val t = LocalTaste.current
    val context = LocalContext.current
    var text by remember { mutableStateOf("") }
    var attach by remember { mutableStateOf<Uri?>(null) }
    var busy by remember { mutableStateOf(false) }

    val picker = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        attach = uri
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = t.surface,
        dragHandle = null,
    ) {
        Column(
            Modifier
                .fillMaxWidth()
                .padding(start = 18.dp, end = 18.dp, top = 20.dp, bottom = 30.dp),
        ) {
            SectionLabel("new save")
            Spacer(Modifier.height(10.dp))
            TasteField(
                value = text,
                onValueChange = { text = it },
                placeholder = "drop a note or paste a link — links save as cards",
                modifier = Modifier.fillMaxWidth(),
                singleLine = false,
                minLines = 4,
            )
            Spacer(Modifier.height(12.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                FlatButton(
                    text = if (attach != null) "image ✓" else "attach image",
                    ghost = true,
                    onClick = { picker.launch("image/*") },
                )
                Spacer(Modifier.weight(1f))
                FlatButton(
                    text = if (busy) "saving…" else "save",
                    enabled = !busy && (text.isNotBlank() || attach != null),
                    onClick = {
                        if (!busy) {
                            busy = true
                            val file = attach?.let {
                                runCatching { uriToCache(context, it) }.getOrNull()
                            }
                            vm.quickSave(
                                text = text,
                                title = "",
                                file = file,
                                mime = "image/*",
                                onDone = {
                                    busy = false
                                    onSaved("saved")
                                },
                                onError = { msg ->
                                    busy = false
                                    Toast.makeText(context, msg, Toast.LENGTH_SHORT).show()
                                },
                            )
                        }
                    },
                )
            }
        }
    }
}
package com.niknair.tasteengine.ui

import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.staggeredgrid.LazyVerticalStaggeredGrid
import androidx.compose.foundation.lazy.staggeredgrid.StaggeredGridCells
import androidx.compose.foundation.lazy.staggeredgrid.items as staggeredItems
import androidx.compose.foundation.lazy.staggeredgrid.rememberLazyStaggeredGridState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.niknair.tasteengine.LocalTaste
import com.niknair.tasteengine.Mori
import com.niknair.tasteengine.net.Api
import com.niknair.tasteengine.net.ClusterGroup
import com.niknair.tasteengine.net.ItemCard
import com.niknair.tasteengine.net.Parse
import com.niknair.tasteengine.vm.VaultVm
import kotlinx.coroutines.launch

/**
 * Groups: ONE similarity cluster at a time, cards only — no tile list, no
 * cluster names. The bottom bar carries the same filters the posts tab has; a
 * filter narrows the current cluster, and when nothing in it matches the server
 * is asked for a cluster that does have matches.
 */
@Composable
fun GroupsScreen(vm: VaultVm, openDetail: (String) -> Unit) {
    val t = LocalTaste.current
    val scope = rememberCoroutineScope()
    val gridState = rememberLazyStaggeredGridState()

    var cards by remember { mutableStateOf<List<ItemCard>>(emptyList()) }
    var cluster by remember { mutableStateOf<ClusterGroup?>(null) }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var showSheet by remember { mutableStateOf(false) }

    var kind by remember { mutableStateOf("") }
    var source by remember { mutableStateOf("") }
    var hue by remember { mutableStateOf("") }
    var tag by remember { mutableStateOf("") }

    // cluster ids already shown, so shuffle keeps handing back fresh ones
    val seen = remember { mutableStateListOf<String>() }

    val filterCount = listOf(kind, source, hue, tag).count { it.isNotEmpty() }

    suspend fun loadRandom() {
        loading = true
        error = null
        try {
            val detail = Parse.groupDetail(
                Api.randomGroup(
                    minSize = 6,
                    limit = 120,
                    exclude = seen.takeLast(12),
                    kind = kind,
                    source = source,
                    hue = hue,
                    tag = tag,
                )
            )
            if (detail.cluster.id.isNotEmpty() && !seen.contains(detail.cluster.id)) {
                seen.add(detail.cluster.id)
            }
            cluster = detail.cluster
            cards = detail.items
        } catch (e: Exception) {
            error = e.message ?: "could not load a cluster"
        } finally {
            loading = false
        }
    }

    /** Keep the cluster, re-slice it by the filters. When the filters leave
     *  nothing behind, jump to a cluster that has matches instead. */
    suspend fun reloadCurrent() {
        val id = cluster?.id
        if (id.isNullOrEmpty()) {
            loadRandom()
            return
        }
        loading = true
        error = null
        try {
            val detail = Parse.groupDetail(
                Api.group(
                    id = id,
                    limit = 120,
                    kind = kind,
                    source = source,
                    hue = hue,
                    tag = tag,
                )
            )
            if (detail.items.isEmpty() && filterCount > 0) {
                loading = false
                loadRandom()
                return
            }
            cluster = detail.cluster
            cards = detail.items
        } catch (e: Exception) {
            error = e.message ?: "could not load this cluster"
        } finally {
            loading = false
        }
    }

    LaunchedEffect(Unit) {
        if (cards.isEmpty()) loadRandom()
    }

    var lastFilterKey by remember { mutableStateOf("") }
    LaunchedEffect(kind, source, hue, tag) {
        val key = "$kind|$source|$hue|$tag"
        if (key == lastFilterKey) return@LaunchedEffect
        lastFilterKey = key
        if (cluster != null || filterCount > 0) reloadCurrent()
    }

    Column(
        Modifier
            .fillMaxSize()
            .background(t.field)
            .statusBarsPadding(),
    ) {
        PullToRefreshBox(
            isRefreshing = loading && cards.isNotEmpty(),
            onRefresh = { scope.launch { loadRandom() } },
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth(),
        ) {
            when {
                cards.isEmpty() && loading -> ShimmerGrid(columns = vm.gridColumns)
                cards.isEmpty() && error != null ->
                    NoteBlock(error ?: "error", retry = { scope.launch { loadRandom() } })
                cards.isEmpty() -> NoteBlock("nothing in this cluster — shuffle for another")
                else -> LazyVerticalStaggeredGrid(
                    columns = StaggeredGridCells.Fixed(vm.gridColumns),
                    state = gridState,
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(start = 12.dp, end = 12.dp, top = 8.dp, bottom = 24.dp),
                    verticalItemSpacing = 10.dp,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    staggeredItems(cards, key = { it.id }) { item ->
                        VaultCard(item) {
                            vm.interact(item.id)
                            openDetail(item.id)
                        }
                    }
                }
            }
        }

        // bottom bar: same height and surfaces as the posts tab
        Row(
            Modifier
                .fillMaxWidth()
                .height(BottomBarHeight)
                .background(t.surface)
                .padding(start = 10.dp, end = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ShuffleBar(
                count = cards.size,
                loading = loading,
                onClick = { scope.launch { loadRandom() } },
                modifier = Modifier
                    .weight(1f)
                    .height(44.dp),
            )
            Spacer(Modifier.width(6.dp))
            ToolIcon(
                active = showSheet || filterCount > 0,
                onClick = { showSheet = true },
                desc = "filters",
            ) {
                GridGlyph(vm.gridColumns, if (showSheet || filterCount > 0) t.accentStrong else t.text)
            }
        }
    }

    if (showSheet) {
        GroupsFilterSheet(
            vm = vm,
            kind = kind,
            source = source,
            hue = hue,
            tag = tag,
            onKind = { kind = it },
            onSource = { source = it },
            onHue = { hue = it },
            onTag = { tag = it },
            onClear = {
                kind = ""
                source = ""
                hue = ""
                tag = ""
            },
            onDismiss = { showSheet = false },
        )
    }
}

/** The wide control in the bottom bar: next cluster, with the current count. */
@Composable
private fun ShuffleBar(
    count: Int,
    loading: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val t = LocalTaste.current
    val label = when {
        loading -> "loading"
        count == 0 -> "shuffle"
        count == 1 -> "1 card · next"
        else -> "$count cards · next"
    }
    Box(
        modifier
            .background(t.canvas)
            .clickable(enabled = !loading) { onClick() }
            .padding(horizontal = 14.dp),
        contentAlignment = Alignment.CenterStart,
    ) {
        Text(
            label,
            color = if (loading) t.muted else t.text,
            fontSize = 12.sp,
            fontFamily = Mori,
            letterSpacing = 0.2.sp,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** Filter sheet: the four filter groups the posts tab uses, plus columns. */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
private fun GroupsFilterSheet(
    vm: VaultVm,
    kind: String,
    source: String,
    hue: String,
    tag: String,
    onKind: (String) -> Unit,
    onSource: (String) -> Unit,
    onHue: (String) -> Unit,
    onTag: (String) -> Unit,
    onClear: () -> Unit,
    onDismiss: () -> Unit,
) {
    val t = LocalTaste.current
    val active = listOf(kind, source, hue, tag).count { it.isNotEmpty() }
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
                SectionLabel("filter this cluster")
                Spacer(Modifier.weight(1f))
                if (active > 0) FlatButton("clear all", ghost = true, onClick = onClear)
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
            FilterGroup("kind", vm.kinds, kind) { onKind(if (it == kind) "" else it) }
            FilterGroup("source", vm.sources, source) { onSource(if (it == source) "" else it) }
            FilterGroup("hue", vm.hues, hue) { onHue(if (it == hue) "" else it) }
            FilterGroup("tag", vm.tags, tag) { onTag(if (it == tag) "" else it) }
        }
    }
}
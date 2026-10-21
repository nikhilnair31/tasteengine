package com.niknair.tasteengine.ui

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
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.staggeredgrid.LazyVerticalStaggeredGrid
import androidx.compose.foundation.lazy.staggeredgrid.StaggeredGridCells
import androidx.compose.foundation.lazy.staggeredgrid.items as staggeredItems
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Done
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import com.niknair.tasteengine.LocalTaste
import com.niknair.tasteengine.AppFont
import com.niknair.tasteengine.net.Api
import com.niknair.tasteengine.net.Followup
import com.niknair.tasteengine.vm.VaultVm

/**
 * Follow-ups use the same card grid as the posts tab: the saved image IS the card,
 * with a small scrim for the title. The done/dismiss actions sit on the card itself.
 */
@Composable
fun FollowupsScreen(vm: VaultVm, openDetail: (String) -> Unit) {
    val t = LocalTaste.current
    LaunchedEffect(Unit) { vm.loadFollowups() }
    var showFilter by remember { mutableStateOf(false) }

    Column(
        Modifier
            .fillMaxSize()
            .background(t.field)
            .statusBarsPadding(),
    ) {
        PullToRefreshBox(
            isRefreshing = vm.followupsLoading && vm.followups.isNotEmpty(),
            onRefresh = { vm.loadFollowups() },
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth(),
        ) {
            Box(
                Modifier
                    .fillMaxSize()
                    .background(t.field),
            ) {
                when {
                    vm.followupsLoading && vm.followups.isEmpty() -> ShimmerGrid(columns = vm.gridColumns)
                    vm.followups.isEmpty() -> NoteBlock(
                        if (vm.followupStatus == "open") "no open follow-ups — everything handled" else "nothing here",
                    )
                    else -> LazyVerticalStaggeredGrid(
                        columns = StaggeredGridCells.Fixed(vm.gridColumns),
                        modifier = Modifier.fillMaxSize(),
                        contentPadding = PaddingValues(start = 12.dp, end = 12.dp, top = 10.dp, bottom = 20.dp),
                        verticalItemSpacing = 10.dp,
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        staggeredItems(vm.followups, key = { it.id }) { f ->
                            FollowupTile(f, vm, openDetail)
                        }
                    }
                }
            }
        }
        Row(
            Modifier
                .fillMaxWidth()
                .height(BottomBarHeight)
                .background(t.surface)
                .padding(horizontal = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                "${vm.followups.size} ${vm.followupStatus}",
                color = t.muted,
                fontSize = 11.sp,
                fontFamily = AppFont,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.widthIn(max = 130.dp),
            )
            Spacer(Modifier.width(6.dp))
            ToolIcon(active = vm.followupStatus != "open", onClick = { showFilter = true }, desc = "followup filter") {
                FunnelGlyph(if (vm.followupStatus != "open") t.accentStrong else t.text)
            }
            Spacer(Modifier.weight(1f))
            ViewControls(vm)
        }
    }

    if (showFilter) {
        FollowupFilterSheet(
            current = vm.followupStatus,
            currentKind = vm.followupKind,
            kinds = vm.followups
                .groupingBy { it.kind }
                .eachCount()
                .entries
                .sortedByDescending { it.value }
                .map { it.key to it.value },
            onDismiss = { showFilter = false },
            onPick = { st ->
                showFilter = false
                vm.loadFollowups(st)
            },
            onPickKind = { k ->
                showFilter = false
                vm.loadFollowups(vm.followupStatus, k)
            },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
private fun FollowupFilterSheet(
    current: String,
    currentKind: String,
    kinds: List<Pair<String, Int>>,
    onDismiss: () -> Unit,
    onPick: (String) -> Unit,
    onPickKind: (String) -> Unit,
) {
    val t = LocalTaste.current
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
            SectionLabel("status")
            Spacer(Modifier.height(6.dp))
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                listOf("open", "done", "dismissed").forEach { st ->
                    Chip(st, active = current == st, count = null) { onPick(st) }
                }
            }
            Spacer(Modifier.height(16.dp))
            SectionLabel("kind")
            Spacer(Modifier.height(6.dp))
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Chip("all", active = currentKind == "", count = null) {
                    onPickKind("")
                }
                kinds.forEach { (k, n) ->
                    Chip(k, active = currentKind == k, count = n) {
                        onPickKind(if (currentKind == k) "" else k)
                    }
                }
            }
        }
    }
}

@Composable
private fun FollowupTile(f: Followup, vm: VaultVm, openDetail: (String) -> Unit) {
    val t = LocalTaste.current
    Box(
        Modifier
            .fillMaxWidth()
            .background(t.surface)
            .border(1.dp, t.line, RoundedCornerShape(0.dp))
            .clickable { if (f.itemId.isNotEmpty()) openDetail(f.itemId) },
    ) {
        Box(
            Modifier
                .fillMaxWidth()
                .aspectRatio(f.aspect)
                .background(t.surface2),
        ) {
            if (f.itemThumb.isNotEmpty()) {
                AsyncImage(
                    model = Api.mediaUrl(f.itemThumb),
                    contentDescription = f.title,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.fillMaxSize(),
                )
            } else {
                Text(
                    f.title.ifEmpty { f.kind }.take(80),
                    color = t.muted,
                    fontSize = 11.sp,
                    fontFamily = AppFont,
                    maxLines = 4,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(9.dp),
                )
            }
            Box(
                Modifier
                    .align(Alignment.TopStart)
                    .background(Color(0xB3120A0D))
                    .padding(horizontal = 5.dp, vertical = 3.dp),
            ) {
                Mark(f.kind, color = Color(0xFFFDF6F6))
            }
            Row(Modifier.align(Alignment.TopEnd)) {
                if (f.status == "open") {
                    TileAction(Icons.Default.Done, "mark done", t.ok) { vm.updateFollowup(f.id, "done") }
                    TileAction(Icons.Default.Close, "dismiss", Color(0xFFFDF6F6)) { vm.updateFollowup(f.id, "dismissed") }
                } else {
                    TileAction(Icons.Default.Refresh, "reopen", Color(0xFFFDF6F6)) { vm.updateFollowup(f.id, "open") }
                }
            }
        }
    }
}

@Composable
private fun TileAction(icon: ImageVector, desc: String, tint: Color, onClick: () -> Unit) {
    Box(
        Modifier
            .size(30.dp)
            .background(Color(0x99120A0D))
            .clickable { onClick() },
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = desc, tint = tint, modifier = Modifier.size(15.dp))
    }
}

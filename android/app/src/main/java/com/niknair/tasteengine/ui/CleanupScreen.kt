package com.niknair.tasteengine.ui

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
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowForward
import androidx.compose.material.icons.filled.Build
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.AlertDialog
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
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import com.niknair.tasteengine.LocalTaste
import com.niknair.tasteengine.Mori
import com.niknair.tasteengine.net.Api
import com.niknair.tasteengine.net.CleanupGroup
import com.niknair.tasteengine.vm.VaultVm

private val channelLabels = listOf(
    "source" to "by source",
    "media" to "by media",
    "empty" to "empty links",
    "all" to "everything",
)

@Composable
fun CleanupScreen(vm: VaultVm, openDetail: (String) -> Unit, onBack: () -> Unit) {
    val t = LocalTaste.current
    val context = LocalContext.current
    var autoChannel by remember { mutableStateOf<String?>(null) }
    var showClean by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { vm.loadCleanup() }

    Column(
        Modifier
            .fillMaxSize()
            .background(t.field)
            .statusBarsPadding(),
    ) {
        PageHeader("cleanup", onBack)

        PullToRefreshBox(
            isRefreshing = vm.cleanupLoading && vm.cleanupGroups.isNotEmpty(),
            onRefresh = { vm.loadCleanup() },
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
                vm.cleanupLoading && vm.cleanupGroups.isEmpty() -> ShimmerList(thumb = thumbFor(vm.gridColumns))
                vm.cleanupGroups.isEmpty() -> NoteBlock("no duplicates found")
                else -> Column(Modifier.fillMaxSize()) {
                    LazyColumn(
                        Modifier.weight(1f),
                        contentPadding = PaddingValues(12.dp),
                        verticalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        itemsIndexed(vm.cleanupGroups) { idx, group ->
                            CleanupGroupCard(idx, group, vm, openDetail)
                        }
                    }
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .height(BottomBarHeight)
                            .background(t.surface)
                            .navigationBarsPadding()
                            .padding(start = 14.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            "scanned ${vm.cleanupScanned} · ${vm.cleanupGroups.size} groups",
                            color = t.muted,
                            fontSize = 11.sp,
                            fontFamily = Mori,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.widthIn(max = 170.dp),
                        )
                        Spacer(Modifier.width(6.dp))
                        if (vm.cleanupSelected.isNotEmpty()) {
                            Text(
                                "${vm.cleanupSelected.size} to delete",
                                color = t.text,
                                fontSize = 13.sp,
                                fontFamily = Mori,
                            )
                            Spacer(Modifier.weight(1f))
                            ToolIcon(onClick = { vm.clearCleanupSelection() }, desc = "clear selection") {
                                Icon(
                                    Icons.Default.Close,
                                    contentDescription = null,
                                    tint = t.text,
                                    modifier = Modifier.size(20.dp),
                                )
                            }
                            ToolIcon(onClick = {
                                vm.cleanupDeleteSelected { n ->
                                    Toast.makeText(context, "deleted $n", Toast.LENGTH_SHORT).show()
                                }
                            }, desc = "delete selected") {
                                Icon(
                                    Icons.Default.Delete,
                                    contentDescription = null,
                                    tint = t.danger,
                                    modifier = Modifier.size(20.dp),
                                )
                            }
                        } else {
                            ToolIcon(onClick = { showClean = true }, desc = "clean options") {
                                Icon(
                                    Icons.Default.Build,
                                    contentDescription = null,
                                    tint = t.text,
                                    modifier = Modifier.size(21.dp),
                                )
                            }
                            Spacer(Modifier.weight(1f))
                        }
                        ViewControls(vm)
                        Spacer(Modifier.width(6.dp))
                    }
                }
            }
        }
        }
    }

    autoChannel?.let { channel ->
        val label = channelLabels.firstOrNull { it.first == channel }?.second ?: channel
        AlertDialog(
            onDismissRequest = { autoChannel = null },
            containerColor = t.surface,
            title = {
                Text("clean duplicates ($label)?", color = t.text, fontFamily = Mori, fontSize = 15.sp)
            },
            text = {
                Text(
                    "keeps the best post in every matching group, moves the rest to trash",
                    color = t.muted,
                    fontFamily = Mori,
                    fontSize = 13.sp,
                )
            },
            confirmButton = {
                FlatButton("delete", danger = true, onClick = {
                    vm.cleanupDeleteType(channel) { n ->
                        Toast.makeText(context, "deleted $n", Toast.LENGTH_SHORT).show()
                    }
                    autoChannel = null
                })
            },
            dismissButton = {
                FlatButton("cancel", ghost = true, onClick = { autoChannel = null })
            },
        )
    }

    if (showClean) {
        CleanSheet(
            onDismiss = { showClean = false },
            onPick = { ch ->
                showClean = false
                autoChannel = ch
            },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
private fun CleanSheet(onDismiss: () -> Unit, onPick: (String) -> Unit) {
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
            SectionLabel("clean duplicates")
            Spacer(Modifier.height(6.dp))
            Text(
                "keeps the best post in each matching group, moves the rest to trash",
                color = t.muted,
                fontSize = 12.sp,
                fontFamily = Mori,
            )
            Spacer(Modifier.height(14.dp))
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                channelLabels.forEach { (value, label) ->
                    Chip(label, active = false, count = null) { onPick(value) }
                }
            }
        }
    }
}

@Composable
private fun CleanupGroupCard(
    index: Int,
    group: CleanupGroup,
    vm: VaultVm,
    openDetail: (String) -> Unit,
) {
    val t = LocalTaste.current
    Column(
        Modifier
            .fillMaxWidth()
            .background(t.surface)
            .border(1.dp, t.line, RoundedCornerShape(0.dp))
            .padding(10.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Mark("group ${index + 1}")
            Spacer(Modifier.width(8.dp))
            group.evidence.forEach { e ->
                Mark(e, color = t.muted)
                Spacer(Modifier.width(6.dp))
            }
            Spacer(Modifier.weight(1f))
            Text(
                "${group.size}",
                color = t.muted,
                fontSize = 11.sp,
                fontFamily = Mori,
            )
        }
        Spacer(Modifier.height(6.dp))
        group.members.forEach { member ->
            val selected = vm.cleanupSelected.contains(member.id)
            val keeper = member.id == group.leader
            Row(
                Modifier
                    .fillMaxWidth()
                    .clickable { vm.toggleCleanupSelection(member.id) }
                    .padding(vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(
                    Modifier
                        .size(18.dp)
                        .background(if (selected) t.accent else Color.Transparent)
                        .border(1.dp, if (selected) t.accentStrong else t.lineStrong, RoundedCornerShape(0.dp)),
                    contentAlignment = Alignment.Center,
                ) {
                    if (selected) {
                        Icon(
                            Icons.Default.Check,
                            contentDescription = null,
                            tint = Color(0xFFFDF6F6),
                            modifier = Modifier.size(13.dp),
                        )
                    }
                }
                Spacer(Modifier.width(10.dp))
                val cover = member.cover
                if (cover != null && cover.best.isNotEmpty()) {
                    AsyncImage(
                        model = Api.mediaUrl(cover.best),
                        contentDescription = null,
                        contentScale = ContentScale.Crop,
                        modifier = Modifier
                            .size(thumbFor(vm.gridColumns))
                            .background(t.surface2)
                            .border(1.dp, t.lineSoft, RoundedCornerShape(0.dp)),
                    )
                    Spacer(Modifier.width(8.dp))
                }
                Column(Modifier.weight(1f)) {
                    Text(
                        member.title.ifEmpty { member.url },
                        color = t.text,
                        fontSize = 12.5.sp,
                        fontFamily = Mori,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Spacer(Modifier.height(2.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Mark(member.source.ifEmpty { member.kind }, color = t.muted)
                        if (keeper) {
                            Spacer(Modifier.width(8.dp))
                            Mark("keep", color = t.ok)
                        }
                    }
                }
                IconButton(onClick = { openDetail(member.id) }) {
                    Icon(
                        Icons.Default.ArrowForward,
                        contentDescription = "open post",
                        tint = t.muted,
                        modifier = Modifier.size(16.dp),
                    )
                }
            }
        }
    }
}
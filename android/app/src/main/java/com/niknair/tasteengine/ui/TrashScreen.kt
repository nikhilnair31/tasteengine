package com.niknair.tasteengine.ui

import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
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
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
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
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import com.niknair.tasteengine.LocalTaste
import com.niknair.tasteengine.AppFont
import com.niknair.tasteengine.net.Api
import com.niknair.tasteengine.net.ItemCard
import com.niknair.tasteengine.vm.VaultVm

@Composable
fun TrashScreen(vm: VaultVm, onBack: () -> Unit) {
    val t = LocalTaste.current
    val context = LocalContext.current
    var destroyTarget by remember { mutableStateOf<ItemCard?>(null) }
    var showPurge by remember { mutableStateOf(false) }
    var showRestoreAll by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { vm.loadTrash() }

    Column(
        Modifier
            .fillMaxSize()
            .background(t.field)
            .statusBarsPadding(),
    ) {
        PageHeader("trash", onBack)

        PullToRefreshBox(
            isRefreshing = vm.trashLoading && vm.trashItems.isNotEmpty(),
            onRefresh = { vm.loadTrash() },
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
                vm.trashLoading && vm.trashItems.isEmpty() -> ShimmerList(thumb = thumbFor(vm.gridColumns))
                vm.trashItems.isEmpty() -> NoteBlock("trash is empty")
                else -> LazyColumn(
                    contentPadding = PaddingValues(12.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(vm.trashItems, key = { it.id }) { item ->
                        TrashRow(
                            item = item,
                            thumb = thumbFor(vm.gridColumns),
                            onRestore = {
                                vm.restoreTrashItem(item.id)
                                Toast.makeText(context, "restored", Toast.LENGTH_SHORT).show()
                            },
                            onDestroy = { destroyTarget = item },
                        )
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
                .navigationBarsPadding()
                .padding(horizontal = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                "auto-purge after ${vm.trashDays} days",
                color = t.muted,
                fontSize = 11.sp,
                fontFamily = AppFont,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.widthIn(max = 150.dp),
            )
            Spacer(Modifier.width(6.dp))
            ToolIcon(onClick = { showRestoreAll = true }, desc = "restore all") {
                Icon(
                    Icons.Default.Refresh,
                    contentDescription = null,
                    tint = t.ok,
                    modifier = Modifier.size(21.dp),
                )
            }
            ToolIcon(onClick = { showPurge = true }, desc = "empty trash") {
                Icon(
                    Icons.Default.Delete,
                    contentDescription = null,
                    tint = t.danger,
                    modifier = Modifier.size(21.dp),
                )
            }
            Spacer(Modifier.weight(1f))
            ViewControls(vm)
        }
    }

    if (destroyTarget != null) {
        AlertDialog(
            onDismissRequest = { destroyTarget = null },
            containerColor = t.surface,
            title = { Text("delete forever?", color = t.text, fontFamily = AppFont, fontSize = 16.sp) },
            text = {
                Text(
                    destroyTarget?.let { it.title.ifEmpty { it.url } } ?: "",
                    color = t.muted,
                    fontFamily = AppFont,
                    fontSize = 13.sp,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                )
            },
            confirmButton = {
                FlatButton("delete", danger = true, onClick = {
                    destroyTarget?.let {
                        vm.destroyTrashItem(it.id)
                        Toast.makeText(context, "deleted", Toast.LENGTH_SHORT).show()
                    }
                    destroyTarget = null
                })
            },
            dismissButton = {
                FlatButton("cancel", ghost = true, onClick = { destroyTarget = null })
            },
        )
    }
    if (showPurge) {
        AlertDialog(
            onDismissRequest = { showPurge = false },
            containerColor = t.surface,
            title = { Text("empty the trash?", color = t.text, fontFamily = AppFont, fontSize = 16.sp) },
            text = {
                Text(
                    "permanently deletes everything in the trash — no undo",
                    color = t.muted,
                    fontFamily = AppFont,
                    fontSize = 13.sp,
                )
            },
            confirmButton = {
                FlatButton("empty", danger = true, onClick = {
                    vm.purgeTrash()
                    Toast.makeText(context, "trash emptied", Toast.LENGTH_SHORT).show()
                    showPurge = false
                })
            },
            dismissButton = {
                FlatButton("cancel", ghost = true, onClick = { showPurge = false })
            },
        )
    }
    if (showRestoreAll) {
        AlertDialog(
            onDismissRequest = { showRestoreAll = false },
            containerColor = t.surface,
            title = { Text("restore everything?", color = t.text, fontFamily = AppFont, fontSize = 16.sp) },
            text = {
                Text(
                    "brings every trashed post back to the vault",
                    color = t.muted,
                    fontFamily = AppFont,
                    fontSize = 13.sp,
                )
            },
            confirmButton = {
                FlatButton("restore all", onClick = {
                    vm.restoreAllTrash()
                    Toast.makeText(context, "restored", Toast.LENGTH_SHORT).show()
                    showRestoreAll = false
                })
            },
            dismissButton = {
                FlatButton("cancel", ghost = true, onClick = { showRestoreAll = false })
            },
        )
    }
}

@Composable
private fun TrashRow(item: ItemCard, thumb: androidx.compose.ui.unit.Dp, onRestore: () -> Unit, onDestroy: () -> Unit) {
    val t = LocalTaste.current
    Row(
        Modifier
            .fillMaxWidth()
            .background(t.surface)
            .border(1.dp, t.line, RoundedCornerShape(0.dp))
            .padding(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val cover = item.cover
        if (cover != null && cover.best.isNotEmpty()) {
            AsyncImage(
                model = Api.mediaUrl(cover.best),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier
                    .size(thumb)
                    .background(t.surface2)
                    .border(1.dp, t.lineSoft, RoundedCornerShape(0.dp)),
            )
            Spacer(Modifier.width(10.dp))
        }
        Column(Modifier.weight(1f)) {
            Text(
                item.title.ifEmpty { item.text.take(60) }.ifEmpty { item.url },
                color = t.text,
                fontSize = 13.sp,
                fontFamily = AppFont,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Spacer(Modifier.height(3.dp))
            Text(
                "deleted ${item.deletedAt.take(10)}",
                color = t.muted,
                fontSize = 11.sp,
                fontFamily = AppFont,
            )
        }
        IconButton(onClick = onRestore) {
            Icon(
                Icons.Default.Refresh,
                contentDescription = "restore",
                tint = t.ok,
                modifier = Modifier.size(19.dp),
            )
        }
        IconButton(onClick = onDestroy) {
            Icon(
                Icons.Default.Delete,
                contentDescription = "delete forever",
                tint = t.danger,
                modifier = Modifier.size(19.dp),
            )
        }
    }
}
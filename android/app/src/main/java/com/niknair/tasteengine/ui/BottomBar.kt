package com.niknair.tasteengine.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Done
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.niknair.tasteengine.LocalTaste
import com.niknair.tasteengine.AppFont

/** The library views: posts, follow-ups, groups, inspire. Trash and cleanup live in settings. */
enum class AppTab(val label: String) {
    POSTS("posts"),
    FOLLOWUPS("followups"),
    GROUPS("groups"),
    INSPIRE("inspire"),
    SETTINGS("settings"),
}

private fun iconFor(tab: AppTab): ImageVector? = when (tab) {
    AppTab.POSTS -> Icons.Default.Home
    AppTab.FOLLOWUPS -> Icons.Default.Done
    AppTab.GROUPS -> null // drawn 2x2 tile glyph
    AppTab.INSPIRE -> null // drawn glyph
    AppTab.SETTINGS -> Icons.Default.Settings
}

@Composable
fun TasteBottomBar(
    current: AppTab,
    onSelect: (AppTab) -> Unit,
    modifier: Modifier = Modifier,
    hairline: Boolean = true,
) {
    val t = LocalTaste.current
    Row(
        modifier
            .fillMaxWidth()
            .background(t.canvas)
            .navigationBarsPadding()
            .then(
                if (hairline) {
                    Modifier.drawBehind {
                        drawLine(
                            color = t.line,
                            start = Offset(0f, 0f),
                            end = Offset(size.width, 0f),
                            strokeWidth = 1.dp.toPx(),
                        )
                    }
                } else {
                    Modifier
                },
            ),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        AppTab.entries.forEach { tab ->
            val active = tab == current
            Column(
                Modifier
                    .weight(1f)
                    .height(TabBarHeight)
                    .clickable { onSelect(tab) }
                    .padding(top = 6.dp, bottom = 6.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                // fixed 20dp slot: an Icon and a drawn glyph must occupy identical bounds,
                // or the bolt tab's label sits lower than the others.
                Box(Modifier.size(20.dp), contentAlignment = Alignment.Center) {
                    val ic = iconFor(tab)
                    if (ic != null) {
                        Icon(
                            imageVector = ic,
                            contentDescription = tab.label,
                            tint = if (active) t.accentStrong else t.muted,
                            modifier = Modifier.size(20.dp),
                        )
                    } else if (tab == AppTab.GROUPS) {
                        GridGlyph(
                            columns = 2,
                            tint = if (active) t.accentStrong else t.muted,
                            modifier = Modifier
                                .size(18.dp)
                                .semantics { contentDescription = tab.label },
                        )
                    } else {
                        BoltGlyph(
                            tint = if (active) t.accentStrong else t.muted,
                            modifier = Modifier.size(18.dp),
                            desc = tab.label,
                        )
                    }
                }
                Spacer(Modifier.height(3.dp))
                Text(
                    tab.label,
                    color = if (active) t.accentStrong else t.muted,
                    fontSize = 9.sp,
                    fontFamily = AppFont,
                    letterSpacing = 0.2.sp,
                )
            }
        }
    }
}

/** Four tabs, icon plus one label line. Kept tight so the grid keeps the screen. */
val TabBarHeight = 56.dp

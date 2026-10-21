package com.niknair.tasteengine

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.compose.ui.Modifier
import com.niknair.tasteengine.ui.AppTab
import com.niknair.tasteengine.ui.CleanupScreen
import com.niknair.tasteengine.ui.DetailScreen
import com.niknair.tasteengine.ui.FollowupsScreen
import com.niknair.tasteengine.ui.GridScreen
import com.niknair.tasteengine.ui.GroupsScreen
import com.niknair.tasteengine.ui.InspireScreen
import com.niknair.tasteengine.ui.LoginScreen
import com.niknair.tasteengine.ui.SettingsScreen
import com.niknair.tasteengine.ui.TasteBottomBar
import com.niknair.tasteengine.ui.TrashScreen
import com.niknair.tasteengine.vm.VaultVm

sealed class Screen {
    data class Detail(val id: String) : Screen()
    data object Trash : Screen()
    data object Cleanup : Screen()
}

/** Lets the share sheet hand a saved item to the main activity. */
object NavBus {
    var pendingOpenId by mutableStateOf<String?>(null)
}

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        Session.init(this)
        NavBus.pendingOpenId = intent?.getStringExtra(EXTRA_ITEM_ID)
        setContent {
            var loggedIn by remember { mutableStateOf(Session.isConfigured()) }
            val vm: VaultVm = viewModel()
            TasteTheme(darkTheme = vm.darkTheme) {
                if (!loggedIn) {
                    LoginScreen(onLoggedIn = { loggedIn = true })
                } else {
                    val stack = remember { mutableStateListOf<Screen>() }
                    var tab by remember { mutableStateOf(AppTab.POSTS) }
                    LaunchedEffect(NavBus.pendingOpenId) {
                        NavBus.pendingOpenId?.let { id ->
                            stack.add(Screen.Detail(id))
                            NavBus.pendingOpenId = null
                        }
                    }
                    LaunchedEffect(Unit) { vm.bootstrap() }
                    BackHandler(enabled = stack.isNotEmpty() || tab != AppTab.POSTS) {
                        if (stack.isNotEmpty()) {
                            stack.removeAt(stack.lastIndex)
                        } else {
                            tab = AppTab.POSTS
                        }
                    }
                    val top = stack.lastOrNull()
                    when (top) {
                        null -> Column(Modifier.fillMaxSize().imePadding()) {
                            Box(Modifier.weight(1f)) {
                                when (tab) {
                                    AppTab.POSTS -> GridScreen(
                                        vm = vm,
                                        openDetail = { vm.primeDetailStub(it); stack.add(Screen.Detail(it)) },
                                    )
                                    AppTab.FOLLOWUPS -> FollowupsScreen(
                                        vm = vm,
                                        openDetail = { vm.primeDetailStub(it); stack.add(Screen.Detail(it)) },
                                    )
                                    AppTab.GROUPS -> GroupsScreen(
                                        vm = vm,
                                        openDetail = { vm.primeDetailStub(it); stack.add(Screen.Detail(it)) },
                                    )
                                    AppTab.INSPIRE -> InspireScreen(
                                        vm = vm,
                                        openDetail = { vm.primeDetailStub(it); stack.add(Screen.Detail(it)) },
                                    )
                                    AppTab.SETTINGS -> SettingsScreen(
                                        onThemeChange = { vm.setDark(it) },
                                        onLogout = { Session.clear(); loggedIn = false },
                                        openTrash = { stack.add(Screen.Trash) },
                                        openCleanup = { stack.add(Screen.Cleanup) },
                                    )
                                }
                            }
                            TasteBottomBar(
                                current = tab,
                                onSelect = { tab = it },
                                hairline = tab != AppTab.POSTS,
                            )
                        }
                        is Screen.Detail -> DetailScreen(
                            vm = vm,
                            itemId = top.id,
                            onBack = { stack.removeAt(stack.lastIndex) },
                            openDetail = { vm.primeDetailStub(it); stack.add(Screen.Detail(it)) },
                        )
                        is Screen.Trash -> TrashScreen(
                            vm = vm,
                            onBack = { stack.removeAt(stack.lastIndex) },
                        )
                        is Screen.Cleanup -> CleanupScreen(
                            vm = vm,
                            openDetail = { vm.primeDetailStub(it); stack.add(Screen.Detail(it)) },
                            onBack = { stack.removeAt(stack.lastIndex) },
                        )
                    }
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        this.intent = intent
        NavBus.pendingOpenId = intent.getStringExtra(EXTRA_ITEM_ID)
    }

    companion object {
        const val EXTRA_ITEM_ID = "item_id"
    }
}
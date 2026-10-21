package com.niknair.tasteengine.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.niknair.tasteengine.LocalTaste
import com.niknair.tasteengine.Mori
import com.niknair.tasteengine.Session
import com.niknair.tasteengine.net.Api
import com.niknair.tasteengine.net.ApiException
import kotlinx.coroutines.launch

@Composable
fun SettingsScreen(
    onThemeChange: (Boolean) -> Unit,
    onLogout: () -> Unit,
    openTrash: () -> Unit = {},
    openCleanup: () -> Unit = {},
) {
    val t = LocalTaste.current
    val scope = rememberCoroutineScope()
    var server by remember { mutableStateOf(Session.serverUrl) }
    var user by remember { mutableStateOf(Session.user) }
    var pass by remember { mutableStateOf(Session.pass) }
    var dark by remember { mutableStateOf(Session.darkTheme) }
    var busy by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf<String?>(null) }

    Column(
        Modifier
            .fillMaxSize()
            .background(t.canvas)
            .statusBarsPadding()
            .verticalScroll(rememberScrollState()),
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .padding(start = 20.dp, top = 18.dp, bottom = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            SectionLabel("settings")
            Spacer(Modifier.weight(1f))
        }
        Column(
            Modifier.padding(horizontal = 20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SectionLabel("connection")
            TasteField(
                value = server,
                onValueChange = { server = it },
                placeholder = "server url",
                modifier = Modifier.fillMaxWidth(),
                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                    keyboardType = KeyboardType.Uri,
                ),
            )
            TasteField(
                value = user,
                onValueChange = { user = it },
                placeholder = "username",
                modifier = Modifier.fillMaxWidth(),
            )
            TasteField(
                value = pass,
                onValueChange = { pass = it },
                placeholder = "password",
                modifier = Modifier.fillMaxWidth(),
                password = true,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                FlatButton("save", onClick = {
                    Session.save(server, user, pass)
                    status = "saved"
                })
                FlatButton(
                    text = if (busy) "testing…" else "test",
                    ghost = true,
                    enabled = !busy,
                    onClick = {
                        if (!busy) {
                            busy = true
                            status = "testing…"
                            Session.save(server, user, pass)
                            scope.launch {
                                status = try {
                                    Api.meta()
                                    "connected ✓"
                                } catch (e: ApiException) {
                                    if (e.code == 401) "wrong login" else "error: ${e.message}"
                                } catch (e: Exception) {
                                    "error: ${e.message ?: "no route"}"
                                }
                                busy = false
                            }
                        }
                    },
                )
            }
            if (status != null) {
                Text(status ?: "", color = t.muted, fontSize = 12.sp, fontFamily = Mori)
            }

            Spacer(Modifier.height(10.dp))
            SectionLabel("library")
            SettingRow(
                title = "trash",
                subtitle = "restore or destroy deleted saves",
                onClick = openTrash,
            )
            SettingRow(
                title = "cleanup",
                subtitle = "review duplicate groups",
                onClick = openCleanup,
            )

            Spacer(Modifier.height(10.dp))
            SectionLabel("appearance")
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("dark theme", color = t.text, fontSize = 13.sp, fontFamily = Mori)
                Spacer(Modifier.weight(1f))
                SquareSwitch(
                    checked = dark,
                    onCheckedChange = {
                        dark = it
                        onThemeChange(it)
                    },
                )
            }

            Spacer(Modifier.height(10.dp))
            SectionLabel("account")
            FlatButton("logout", danger = true, onClick = {
                Session.clear()
                onLogout()
            })

            Spacer(Modifier.height(28.dp))
            Text(
                "taste engine android · v0.9.3",
                color = t.muted,
                fontSize = 11.sp,
                fontFamily = Mori,
            )
            Spacer(Modifier.height(30.dp))
        }
    }
}

/** One tappable settings row: title, quiet subtitle, hairline card. */
@Composable
private fun SettingRow(title: String, subtitle: String, onClick: () -> Unit) {
    val t = LocalTaste.current
    Row(
        Modifier
            .fillMaxWidth()
            .background(t.surface2)
            .border(1.dp, t.lineSoft, RoundedCornerShape(0.dp))
            .clickable { onClick() }
            .padding(horizontal = 12.dp, vertical = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, color = t.text, fontSize = 14.sp, fontFamily = Mori, fontWeight = FontWeight.Medium)
            Spacer(Modifier.height(2.dp))
            Text(subtitle, color = t.muted, fontSize = 11.sp, fontFamily = Mori)
        }
        Text("›", color = t.muted, fontSize = 18.sp, fontFamily = Mori)
    }
}

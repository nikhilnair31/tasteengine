package com.niknair.tasteengine.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.niknair.tasteengine.LocalTaste
import com.niknair.tasteengine.Mori
import com.niknair.tasteengine.R
import com.niknair.tasteengine.Session
import com.niknair.tasteengine.net.Api
import com.niknair.tasteengine.net.ApiException
import kotlinx.coroutines.launch

/** Login: same username/password as the web app (HTTP Basic). */
@Composable
fun LoginScreen(onLoggedIn: () -> Unit) {
    val t = LocalTaste.current
    var server by remember { mutableStateOf(Session.serverUrl) }
    var user by remember { mutableStateOf(Session.user) }
    var pass by remember { mutableStateOf(Session.pass) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    Box(
        Modifier
            .fillMaxSize()
            .background(t.field)
            .statusBarsPadding()
            .navigationBarsPadding()
            .imePadding(),
    ) {
        Column(
            modifier = Modifier
                .align(Alignment.Center)
                .fillMaxWidth()
                .widthIn(max = 420.dp)
                .padding(horizontal = 30.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Image(
                painter = painterResource(R.drawable.logo_alt),
                contentDescription = "taste engine",
                modifier = Modifier.size(112.dp),
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = "personal vault",
                color = t.fieldMuted,
                fontSize = 11.sp,
                fontFamily = Mori,
                letterSpacing = 2.sp,
            )
            Spacer(Modifier.height(28.dp))
            TasteField(
                value = server,
                onValueChange = { server = it },
                placeholder = "server url",
                modifier = Modifier.fillMaxWidth(),
                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = KeyboardType.Uri),
            )
            Spacer(Modifier.height(10.dp))
            TasteField(
                value = user,
                onValueChange = { user = it },
                placeholder = "username",
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(10.dp))
            TasteField(
                value = pass,
                onValueChange = { pass = it },
                placeholder = "password",
                modifier = Modifier.fillMaxWidth(),
                password = true,
            )
            if (error != null) {
                Spacer(Modifier.height(12.dp))
                Text(
                    text = error ?: "",
                    color = t.fieldMuted,
                    fontSize = 12.sp,
                    fontFamily = Mori,
                    textAlign = TextAlign.Center,
                )
            }
            Spacer(Modifier.height(20.dp))
            FlatButton(
                text = if (busy) "signing in…" else "sign in",
                modifier = Modifier.fillMaxWidth(),
                enabled = !busy,
                onClick = {
                    if (!busy) {
                        busy = true
                        error = null
                        Session.save(server, user, pass)
                        scope.launch {
                            try {
                                Api.meta()
                                onLoggedIn()
                            } catch (e: ApiException) {
                                error = if (e.code == 401) {
                                    "wrong username or password"
                                } else {
                                    "server error: ${e.message}"
                                }
                            } catch (e: Exception) {
                                error = "cannot reach server: ${e.message ?: "connection failed"}"
                            } finally {
                                busy = false
                            }
                        }
                    }
                },
            )
        }
    }
}
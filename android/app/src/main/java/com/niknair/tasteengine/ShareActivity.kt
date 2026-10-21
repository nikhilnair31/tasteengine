package com.niknair.tasteengine

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.IntentCompat
import androidx.lifecycle.lifecycleScope
import com.niknair.tasteengine.net.Api
import com.niknair.tasteengine.ui.FlatButton
import kotlinx.coroutines.launch
import java.io.File

/**
 * Share-sheet target: any app can send links, text, or images here and the
 * vault saves them through the same API the web app uses.
 */
class ShareActivity : ComponentActivity() {

    data class ShareState(
        val phase: String = "working", // working | done | error | needsLogin
        val message: String = "saving to taste engine…",
        val itemId: String? = null,
    )

    private var state by mutableStateOf(ShareState())

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        Session.init(this)
        setContent {
            TasteTheme(darkTheme = Session.darkTheme) {
                ShareSheet(state = state, onOpen = { openAndFinish() }, onClose = { finish() })
            }
        }
        if (!Session.isConfigured()) {
            state = ShareState(phase = "needsLogin", message = "sign in to taste engine first")
            return
        }
        handleShare()
    }

    private fun openAndFinish() {
        val i = Intent(this, MainActivity::class.java).apply {
            addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            state.itemId?.let { putExtra(MainActivity.EXTRA_ITEM_ID, it) }
        }
        startActivity(i)
        finish()
    }

    private fun handleShare() {
        lifecycleScope.launch {
            try {
                val i = intent ?: return@launch
                val action = i.action ?: Intent.ACTION_SEND
                val type = i.type ?: "text/plain"
                when {
                    action == Intent.ACTION_SEND && type.startsWith("image/") -> {
                        val uri = IntentCompat.getParcelableExtra(i, Intent.EXTRA_STREAM, Uri::class.java)
                            ?: throw IllegalStateException("no image found in share")
                        val copy = copyToCache(uri)
                        val title = i.getStringExtra(Intent.EXTRA_TITLE)
                            ?: i.getStringExtra(Intent.EXTRA_SUBJECT) ?: ""
                        val text = i.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString() ?: ""
                        val resp = Api.uploadImage(copy.file, copy.name, copy.mime, firstUrl(text), title)
                        finishSaved(resp, "saved to taste engine")
                    }
                    action == Intent.ACTION_SEND_MULTIPLE && type.startsWith("image/") -> {
                        val uris = IntentCompat.getParcelableArrayListExtra(i, Intent.EXTRA_STREAM, Uri::class.java)
                            ?: throw IllegalStateException("no images found in share")
                        val title = i.getStringExtra(Intent.EXTRA_SUBJECT) ?: ""
                        var count = 0
                        var first: String? = null
                        for (u in uris.take(8)) {
                            val copy = copyToCache(u)
                            val resp = Api.uploadImage(copy.file, copy.name, copy.mime, "", title)
                            count++
                            if (first == null) first = resp.optString("id").ifEmpty { null }
                        }
                        state = ShareState(phase = "done", message = "saved $count image(s) to taste engine", itemId = first)
                    }
                    else -> {
                        val text = i.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString() ?: ""
                        val subject = i.getStringExtra(Intent.EXTRA_SUBJECT) ?: ""
                        if (text.isBlank() && subject.isBlank()) {
                            throw IllegalStateException("nothing to save")
                        }
                        val resp = Api.saveNote(text = text, title = subject)
                        finishSaved(resp, "saved to taste engine")
                    }
                }
            } catch (e: Exception) {
                state = ShareState(phase = "error", message = e.message ?: "save failed")
            }
        }
    }

    private fun finishSaved(resp: org.json.JSONObject, okMessage: String) {
        val id = resp.optString("id").ifEmpty { null }
        state = ShareState(phase = "done", message = okMessage, itemId = id)
    }

    private fun copyToCache(uri: Uri): Copy {
        val mime = contentResolver.getType(uri) ?: "image/jpeg"
        val ext = when {
            mime.contains("png") -> ".png"
            mime.contains("webp") -> ".webp"
            mime.contains("gif") -> ".gif"
            else -> ".jpg"
        }
        val file = File(cacheDir, "share_${System.currentTimeMillis()}$ext")
        contentResolver.openInputStream(uri)?.use { input ->
            file.outputStream().use { output -> input.copyTo(output) }
        } ?: throw IllegalStateException("cannot read shared image")
        return Copy(file, "shared$ext", mime)
    }

    private data class Copy(val file: File, val name: String, val mime: String)

    private fun firstUrl(text: String): String {
        val m = Regex("https?://\\S+").find(text) ?: return ""
        return m.value.trimEnd('.', ',', ';', ':', '!', '?', ')', ']', '}')
    }
}

@Composable
private fun ShareSheet(state: ShareActivity.ShareState, onOpen: () -> Unit, onClose: () -> Unit) {
    val t = LocalTaste.current
    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Color(0x99000000)),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            modifier = Modifier
                .width(300.dp)
                .background(t.surface)
                .border(1.dp, t.lineStrong, RoundedCornerShape(0.dp))
                .padding(20.dp),
        ) {
            Text(
                text = "taste engine",
                fontSize = 14.sp,
                fontFamily = AppFont,
                fontWeight = FontWeight.SemiBold,
                letterSpacing = 1.4.sp,
                color = t.accentStrong,
            )
            Spacer(Modifier.height(12.dp))
            Text(text = state.message, fontSize = 14.sp, color = t.text)
            Spacer(Modifier.height(16.dp))
            when (state.phase) {
                "working" -> CircularProgressIndicator(
                    color = t.accentStrong,
                    strokeWidth = 2.dp,
                    modifier = Modifier.size(22.dp),
                )
                "done" -> Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    FlatButton("open", onClick = onOpen)
                    FlatButton("close", onClick = onClose, ghost = true)
                }
                "needsLogin" -> Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    FlatButton("open taste engine", onClick = onOpen)
                    FlatButton("close", onClick = onClose, ghost = true)
                }
                else -> Column {
                    Text(
                        text = "check the server address / login in the app",
                        fontSize = 12.sp,
                        color = t.danger,
                    )
                    Spacer(Modifier.height(12.dp))
                    FlatButton("close", onClick = onClose, ghost = true)
                }
            }
        }
    }
}
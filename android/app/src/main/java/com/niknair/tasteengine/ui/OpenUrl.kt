package com.niknair.tasteengine.ui

import android.content.Context
import android.content.Intent
import android.net.Uri
import okhttp3.OkHttpClient
import okhttp3.Request
import java.util.concurrent.TimeUnit

/**
 * Open a saved link. Known domains open directly in their app when it is
 * installed (falls back to the browser); redirect-only hosts are resolved
 * first so the final domain can claim the link.
 */

private val redirectorHosts = setOf("t.co", "bit.ly", "tinyurl.com", "buff.ly", "ow.ly")

private val resolveClient = OkHttpClient.Builder()
    .connectTimeout(4, TimeUnit.SECONDS)
    .readTimeout(4, TimeUnit.SECONDS)
    .followRedirects(true)
    .followSslRedirects(true)
    .build()

private fun hostIs(host: String, vararg domains: String): Boolean =
    domains.any { host == it || host.endsWith(".$it") }

private fun appPackageFor(host: String): String? = when {
    hostIs(host, "x.com", "twitter.com") -> "com.twitter.android"
    hostIs(host, "instagram.com") -> "com.instagram.android"
    hostIs(host, "reddit.com", "redd.it") -> "com.reddit.frontpage"
    hostIs(host, "youtube.com", "youtu.be") -> "com.google.android.youtube"
    hostIs(host, "pinterest.com") -> "com.pinterest"
    hostIs(host, "spotify.com", "spotify.link") -> "com.spotify.music"
    hostIs(host, "tiktok.com") -> "com.zhiliaoapp.musically"
    hostIs(host, "linkedin.com") -> "com.linkedin.android"
    hostIs(host, "steamcommunity.com", "steampowered.com") -> "com.valvesoftware.android.steam.community"
    hostIs(host, "github.com") -> "com.github.android"
    hostIs(host, "amazon.com") -> "com.amazon.mShop.android.shopping"
    else -> null
}

fun openUrl(context: Context, url: String) {
    if (url.isBlank()) return
    val uri = runCatching { Uri.parse(url) }.getOrNull() ?: return
    val host = (uri.host ?: "").lowercase()
    if (host in redirectorHosts) {
        Thread {
            val resolved = runCatching {
                val req = Request.Builder().url(url).get().build()
                resolveClient.newCall(req).execute().use { it.request.url.toString() }
            }.getOrNull()
            openDirect(context, resolved ?: url)
        }.start()
    } else {
        openDirect(context, url)
    }
}

private fun openDirect(context: Context, url: String) {
    val uri = runCatching { Uri.parse(url) }.getOrNull() ?: return
    val host = (uri.host ?: "").lowercase()
    val pkg = appPackageFor(host)
    if (pkg != null) {
        val appIntent = Intent(Intent.ACTION_VIEW, uri).setPackage(pkg)
        if (appIntent.resolveActivity(context.packageManager) != null) {
            val ok = runCatching { context.startActivity(appIntent) }.isSuccess
            if (ok) return
        }
    }
    runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, uri)) }
}

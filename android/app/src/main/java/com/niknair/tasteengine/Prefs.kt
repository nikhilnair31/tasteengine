package com.niknair.tasteengine

import android.content.Context
import android.content.SharedPreferences

/** Login + preference storage. Single user, same Basic auth as the web app. */
object Session {
    // Set your own server on the login screen; this default is local-only.
    const val DEFAULT_SERVER = "http://localhost:8770"

    private var sp: SharedPreferences? = null

    var serverUrl: String = DEFAULT_SERVER
        private set
    var user: String = ""
        private set
    var pass: String = ""
        private set
    var darkTheme: Boolean = true
        private set
    var gridColumns: Int = 2
        private set

    fun init(context: Context) {
        if (sp != null) return
        val p = context.applicationContext.getSharedPreferences("taste", Context.MODE_PRIVATE)
        sp = p
        serverUrl = p.getString("server", DEFAULT_SERVER)?.ifBlank { DEFAULT_SERVER } ?: DEFAULT_SERVER
        user = p.getString("user", "") ?: ""
        pass = p.getString("pass", "") ?: ""
        darkTheme = p.getBoolean("dark", true)
        gridColumns = p.getInt("gridColumns", 2).coerceIn(1, 4)
    }

    fun save(server: String, u: String, pw: String) {
        serverUrl = server.trim().trimEnd('/').ifBlank { DEFAULT_SERVER }
        user = u.trim()
        pass = pw
        sp?.edit()?.putString("server", serverUrl)?.putString("user", user)?.putString("pass", pass)?.apply()
    }

    fun setDark(dark: Boolean) {
        darkTheme = dark
        sp?.edit()?.putBoolean("dark", dark)?.apply()
    }

    fun setGridColumns(n: Int) {
        gridColumns = n.coerceIn(1, 4)
        sp?.edit()?.putInt("gridColumns", gridColumns)?.apply()
    }

    fun clear() {
        user = ""
        pass = ""
        sp?.edit()?.remove("user")?.remove("pass")?.apply()
    }

    fun isConfigured(): Boolean = user.isNotEmpty() && pass.isNotEmpty() && serverUrl.isNotEmpty()
}

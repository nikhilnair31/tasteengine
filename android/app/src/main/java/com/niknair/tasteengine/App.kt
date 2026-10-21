package com.niknair.tasteengine

import android.app.Application
import coil.Coil
import coil.ImageLoader
import okhttp3.Credentials
import okhttp3.OkHttpClient
import java.util.concurrent.TimeUnit

/**
 * Application entry. Sets up the session store and a Coil image loader that
 * carries the Basic-auth header so every /media/ request authenticates.
 */
class App : Application() {
    override fun onCreate() {
        super.onCreate()
        Session.init(this)

        val client = OkHttpClient.Builder()
            .connectTimeout(20, TimeUnit.SECONDS)
            .readTimeout(60, TimeUnit.SECONDS)
            .addInterceptor { chain ->
                val request = if (Session.user.isNotEmpty()) {
                    chain.request().newBuilder()
                        .header("Authorization", Credentials.basic(Session.user, Session.pass))
                        .build()
                } else {
                    chain.request()
                }
                chain.proceed(request)
            }
            .build()

        Coil.setImageLoader(
            ImageLoader.Builder(this)
                .okHttpClient(client)
                .crossfade(false)
                .build()
        )
    }
}
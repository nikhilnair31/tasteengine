package com.niknair.tasteengine.net

import com.niknair.tasteengine.Session
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.Credentials
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MultipartBody
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody
import okhttp3.RequestBody.Companion.asRequestBody
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.URLEncoder
import java.util.concurrent.TimeUnit

class ApiException(val code: Int, message: String) : Exception(message)

/** Thin HTTP client for the Taste Engine server. Basic auth, org.json parsing. */
object Api {
    private val JSON = "application/json; charset=utf-8".toMediaType()

    val client: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(20, TimeUnit.SECONDS)
            .readTimeout(60, TimeUnit.SECONDS)
            .writeTimeout(180, TimeUnit.SECONDS)
            .build()
    }

    fun base(): String = Session.serverUrl.trim().trimEnd('/')

    fun mediaUrl(name: String): String =
        base() + "/media/" + URLEncoder.encode(name, "UTF-8")

    private fun buildRequest(
        path: String,
        method: String = "GET",
        body: RequestBody? = null,
        query: Map<String, String> = emptyMap(),
    ): Request {
        val b = base().toHttpUrlOrNull()?.newBuilder()
            ?: throw ApiException(0, "invalid server url: ${base()}")
        b.encodedPath("/" + path.trim('/'))
        for ((k, v) in query) if (v.isNotEmpty()) b.addQueryParameter(k, v)
        val req = Request.Builder().url(b.build())
        if (Session.user.isNotEmpty()) {
            req.header("Authorization", Credentials.basic(Session.user, Session.pass))
        }
        when (method) {
            "POST" -> req.post(body ?: ByteArray(0).toRequestBody(JSON))
            "PUT" -> req.put(body ?: ByteArray(0).toRequestBody(JSON))
            "DELETE" -> req.delete()
        }
        return req.build()
    }

    private fun exec(request: Request): JSONObject {
        client.newCall(request).execute().use { resp ->
            val text = resp.body?.string() ?: ""
            if (!resp.isSuccessful) {
                throw ApiException(resp.code, text.take(400).ifBlank { "http ${resp.code}" })
            }
            if (text.isBlank()) return JSONObject()
            return try {
                JSONObject(text)
            } catch (e: Exception) {
                throw ApiException(resp.code, "bad json: ${text.take(120)}")
            }
        }
    }

    private fun mimeOrDefault(mime: String) =
        mime.ifBlank { "application/octet-stream" }.toMediaType()

    private fun jsonPayload(vararg pairs: Pair<String, Any?>): RequestBody {
        val o = JSONObject()
        for ((k, v) in pairs) o.put(k, v)
        return o.toString().toRequestBody(JSON)
    }

    // ---------------------------------------------------------------- reads

    suspend fun meta(): JSONObject = withContext(Dispatchers.IO) { exec(buildRequest("api/meta")) }

    suspend fun items(
        offset: Int,
        limit: Int = 48,
        kind: String = "",
        source: String = "",
        hue: String = "",
        tag: String = "",
        q: String = "",
        sort: String = "random",
        seed: Int = 0,
    ): JSONObject = withContext(Dispatchers.IO) {
        exec(
            buildRequest(
                "api/items",
                query = mapOf(
                    "offset" to offset.toString(),
                    "limit" to limit.toString(),
                    "kind" to kind,
                    "source" to source,
                    "hue" to hue,
                    "tag" to tag,
                    "q" to q,
                    "sort" to sort,
                    "seed" to seed.toString(),
                ),
            )
        )
    }

    suspend fun search(
        q: String,
        limit: Int = 60,
        offset: Int = 0,
        kind: String = "",
        source: String = "",
        hue: String = "",
        tag: String = "",
    ): JSONObject = withContext(Dispatchers.IO) {
        exec(
            buildRequest(
                "api/search",
                query = mapOf(
                    "q" to q,
                    "limit" to limit.toString(),
                    "offset" to offset.toString(),
                    "kind" to kind,
                    "source" to source,
                    "hue" to hue,
                    "tag" to tag,
                ),
            )
        )
    }

    suspend fun item(id: String): JSONObject = withContext(Dispatchers.IO) {
        exec(buildRequest("api/items/$id"))
    }

    suspend fun followups(status: String = "open", kind: String = ""): JSONObject =
        withContext(Dispatchers.IO) {
            exec(
                buildRequest(
                    "api/followups",
                    query = mapOf("status" to status, "kind" to kind),
                )
            )
        }

    suspend fun trash(): JSONObject = withContext(Dispatchers.IO) {
        exec(buildRequest("api/trash"))
    }

    suspend fun cleanupGroups(): JSONObject = withContext(Dispatchers.IO) {
        exec(buildRequest("api/cleanup/groups"))
    }

    // ---------------------------------------------------------------- groups

    /** All similarity groups, biggest first. Each entry carries cover item ids. */
    suspend fun groups(minSize: Int = 4, limit: Int = 200): JSONObject =
        withContext(Dispatchers.IO) {
            exec(
                buildRequest(
                    "api/groups",
                    query = mapOf(
                        "min_size" to minSize.toString(),
                        "limit" to limit.toString(),
                    ),
                )
            )
        }

    /** One group the caller has not seen lately, with its items. Filters are
     *  the same ones the posts grid uses; with a filter on, the server skips
     *  clusters that would come back empty. */
    suspend fun randomGroup(
        minSize: Int = 8,
        limit: Int = 24,
        exclude: List<String> = emptyList(),
        kind: String = "",
        source: String = "",
        hue: String = "",
        tag: String = "",
    ): JSONObject = withContext(Dispatchers.IO) {
        exec(
            buildRequest(
                "api/groups/random",
                query = mapOf(
                    "min_size" to minSize.toString(),
                    "limit" to limit.toString(),
                    "exclude" to exclude.joinToString(","),
                    "kind" to kind,
                    "source" to source,
                    "hue" to hue,
                    "tag" to tag,
                ),
            )
        )
    }

    /** One group by id, with its items as cards, optionally filtered. */
    suspend fun group(
        id: String,
        limit: Int = 48,
        kind: String = "",
        source: String = "",
        hue: String = "",
        tag: String = "",
    ): JSONObject = withContext(Dispatchers.IO) {
        exec(
            buildRequest(
                "api/groups/$id",
                query = mapOf(
                    "limit" to limit.toString(),
                    "kind" to kind,
                    "source" to source,
                    "hue" to hue,
                    "tag" to tag,
                ),
            )
        )
    }

    // ---------------------------------------------------------------- inspire

    /** Build a brief from an ask. Empty ask = the server invents one from the vault. */
    suspend fun inspireStart(about: String, seedIds: List<String> = emptyList()): JSONObject =
        withContext(Dispatchers.IO) {
            val seeds = JSONArray()
            seedIds.forEach { seeds.put(it) }
            exec(
                buildRequest(
                    "api/inspire",
                    method = "POST",
                    body = jsonPayload(
                        "mode" to "brief",
                        "project" to "mixed",
                        "platform" to "any",
                        "about" to about,
                        "seed_ids" to seeds,
                    ),
                )
            )
        }

    suspend fun inspireStatus(jobId: String): JSONObject = withContext(Dispatchers.IO) {
        exec(buildRequest("api/inspire/$jobId"))
    }

    /** Plain-markdown brief, for the copy button. */
    suspend fun briefMarkdown(jobId: String): String = withContext(Dispatchers.IO) {
        val req = buildRequest("api/inspire/$jobId/brief.md")
        client.newCall(req).execute().use { resp ->
            val text = resp.body?.string() ?: ""
            if (!resp.isSuccessful) throw ApiException(resp.code, text.take(300))
            text
        }
    }

    // ---------------------------------------------------------------- writes

    suspend fun interact(id: String) {
        withContext(Dispatchers.IO) {
            runCatching { exec(buildRequest("api/items/$id/interact", method = "POST")) }
        }
    }

    suspend fun deleteItem(id: String) {
        withContext(Dispatchers.IO) { exec(buildRequest("api/items/$id", method = "DELETE")) }
    }

    suspend fun refreshItem(id: String) {
        withContext(Dispatchers.IO) { exec(buildRequest("api/items/$id/refresh", method = "POST")) }
    }

    suspend fun explainItem(id: String) {
        withContext(Dispatchers.IO) { exec(buildRequest("api/items/$id/explain", method = "POST")) }
    }

    suspend fun setFollowupStatus(id: String, status: String): JSONObject =
        withContext(Dispatchers.IO) {
            exec(buildRequest("api/followups/$id", method = "POST", body = jsonPayload("status" to status)))
        }

    suspend fun restoreTrash(id: String) {
        withContext(Dispatchers.IO) { exec(buildRequest("api/trash/$id/restore", method = "POST")) }
    }

    suspend fun destroyTrash(id: String) {
        withContext(Dispatchers.IO) { exec(buildRequest("api/trash/$id/destroy", method = "POST")) }
    }

    suspend fun purgeTrash(): JSONObject = withContext(Dispatchers.IO) {
        exec(buildRequest("api/trash/purge", method = "POST"))
    }

    suspend fun restoreAllTrash(): JSONObject = withContext(Dispatchers.IO) {
        exec(buildRequest("api/trash/restore-all", method = "POST"))
    }

    suspend fun cleanupDelete(ids: List<String>): JSONObject = withContext(Dispatchers.IO) {
        exec(
            buildRequest(
                "api/cleanup/delete",
                method = "POST",
                body = jsonPayload("ids" to JSONArray(ids)),
            )
        )
    }

    suspend fun cleanupDeleteType(channel: String): JSONObject = withContext(Dispatchers.IO) {
        exec(
            buildRequest(
                "api/cleanup/delete-type",
                method = "POST",
                body = jsonPayload("channel" to channel),
            )
        )
    }

    suspend fun saveNote(text: String, title: String = ""): JSONObject = withContext(Dispatchers.IO) {
        val body = MultipartBody.Builder()
            .setType(MultipartBody.FORM)
            .addFormDataPart("text", text)
            .addFormDataPart("title", title)
            .build()
        exec(buildRequest("api/notes", method = "POST", body = body))
    }

    suspend fun saveNoteWithFile(
        text: String,
        title: String,
        file: File,
        mime: String,
    ): JSONObject = withContext(Dispatchers.IO) {
        val body = MultipartBody.Builder()
            .setType(MultipartBody.FORM)
            .addFormDataPart("text", text)
            .addFormDataPart("title", title)
            .addFormDataPart("files", file.name, file.asRequestBody(mimeOrDefault(mime)))
            .build()
        exec(buildRequest("api/notes", method = "POST", body = body))
    }

    suspend fun uploadImage(
        file: File,
        name: String,
        mime: String,
        url: String,
        title: String,
    ): JSONObject = withContext(Dispatchers.IO) {
        val payload = JSONObject()
            .put("url", url)
            .put("title", title)
            .put("tags", JSONArray())
            .toString()
        val body = MultipartBody.Builder()
            .setType(MultipartBody.FORM)
            .addFormDataPart("payload", payload)
            .addFormDataPart("file", name.ifBlank { "shared-image" }, file.asRequestBody(mimeOrDefault(mime)))
            .build()
        exec(buildRequest("api/items/upload", method = "POST", body = body))
    }

}
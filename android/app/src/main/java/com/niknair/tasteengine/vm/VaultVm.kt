package com.niknair.tasteengine.vm

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.niknair.tasteengine.Session
import com.niknair.tasteengine.net.Api
import com.niknair.tasteengine.net.ApiException
import com.niknair.tasteengine.net.Brief
import com.niknair.tasteengine.net.BriefParse
import com.niknair.tasteengine.net.CleanupGroup
import com.niknair.tasteengine.net.Followup
import com.niknair.tasteengine.net.FrameVaultHit
import com.niknair.tasteengine.net.ItemCard
import com.niknair.tasteengine.net.Parse
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.io.File
import kotlin.random.Random

data class GridState(
    val items: List<ItemCard> = emptyList(),
    val loading: Boolean = false,
    val refreshing: Boolean = false,
    val hasMore: Boolean = true,
    val total: Int = 0,
    val kind: String = "",
    val source: String = "",
    val hue: String = "",
    val tag: String = "",
    val q: String = "",
    val sort: String = "random",
    val seed: Int = Random.nextInt(),
    val offset: Int = 0,
    val error: String? = null,
    val authError: Boolean = false,
)

class VaultVm(app: Application) : AndroidViewModel(app) {
    companion object {
        const val PAGE = 48
        const val SEARCH_LIMIT = 60
    }

    // ------------------------------------------------------------- vault grid
    var grid by mutableStateOf(GridState())
        private set
    var kinds by mutableStateOf<List<Pair<String, Int>>>(emptyList())
        private set
    var sources by mutableStateOf<List<Pair<String, Int>>>(emptyList())
        private set
    var hues by mutableStateOf<List<Pair<String, Int>>>(emptyList())
        private set
    var tags by mutableStateOf<List<Pair<String, Int>>>(emptyList())
        private set
    var gridColumns by mutableStateOf(Session.gridColumns)
        private set

    // -------------------------------------------------------------- followups
    var followups by mutableStateOf<List<Followup>>(emptyList())
        private set
    var followupStatus by mutableStateOf("open")
        private set
    var followupsLoading by mutableStateOf(false)
        private set

    // ------------------------------------------------------------------ trash
    var trashItems by mutableStateOf<List<ItemCard>>(emptyList())
        private set
    var trashDays by mutableStateOf(30)
        private set
    var trashLoading by mutableStateOf(false)
        private set

    // ---------------------------------------------------------------- cleanup
    var cleanupGroups by mutableStateOf<List<CleanupGroup>>(emptyList())
        private set
    var cleanupScanned by mutableStateOf(0)
        private set
    var cleanupLoading by mutableStateOf(false)
        private set
    var cleanupSelected by mutableStateOf<Set<String>>(emptySet())
        private set

    private var bootstrapped = false

    fun bootstrap() {
        if (bootstrapped) return
        bootstrapped = true
        viewModelScope.launch {
            runCatching { Api.meta() }.onSuccess { m ->
                kinds = pairs(m, "kinds")
                sources = pairs(m, "sources")
                hues = pairs(m, "hues")
                tags = pairs(m, "tags").take(60)
            }
            load(reset = true)
            loadFollowups("open")
        }
    }

    // ================================================================= grid

    fun reload() {
        grid = grid.copy(seed = Random.nextInt())
        load(reset = true)
    }

    fun setKind(kind: String) {
        grid = grid.copy(kind = kind, offset = 0)
        load(reset = true)
    }

    fun setSource(source: String) {
        grid = grid.copy(source = source, offset = 0)
        load(reset = true)
    }

    fun setHue(hue: String) {
        grid = grid.copy(hue = hue, offset = 0)
        load(reset = true)
    }

    fun setTag(tag: String) {
        grid = grid.copy(tag = tag, offset = 0)
        load(reset = true)
    }

    fun clearFilters() {
        grid = grid.copy(kind = "", source = "", hue = "", tag = "", offset = 0)
        load(reset = true)
    }

    fun activeFilterCount(): Int =
        listOf(grid.kind, grid.source, grid.hue, grid.tag).count { it.isNotEmpty() }

    fun setSort(sort: String) {
        grid = grid.copy(sort = sort, offset = 0, seed = Random.nextInt())
        load(reset = true)
    }

    fun changeGridColumns(n: Int) {
        Session.setGridColumns(n)
        gridColumns = Session.gridColumns
    }

    /** Flip 2 -> 3 -> 4 -> 1 -> 2 (regular -> dense -> denser -> exploded). */
    fun cycleGridColumns() {
        val order = listOf(2, 3, 4, 1)
        val i = order.indexOf(gridColumns).let { if (it < 0) 0 else it }
        changeGridColumns(order[(i + 1) % order.size])
    }

    /** Cache the tapped card so the detail hero paints instantly. */
    var detailStub by mutableStateOf<ItemCard?>(null)
        private set

    fun primeDetailStub(id: String) {
        detailStub = grid.items.firstOrNull { it.id == id }
    }

    var darkTheme by mutableStateOf(Session.darkTheme)
        private set

    fun setDark(d: Boolean) {
        Session.setDark(d)
        darkTheme = d
    }

    fun toggleTheme() {
        setDark(!darkTheme)
    }

    fun submitSearch(q: String) {
        grid = grid.copy(q = q.trim(), offset = 0)
        load(reset = true)
    }

    fun clearSearch() {
        submitSearch("")
    }

    fun loadMore() {
        val s = grid
        if (s.loading || !s.hasMore) return
        load(reset = false)
    }

    fun load(reset: Boolean) {
        val s = grid
        if (s.loading && !reset) return
        val offset = if (reset) 0 else s.offset
        val page = if (s.q.isNotEmpty()) SEARCH_LIMIT else PAGE
        grid = s.copy(loading = true, refreshing = reset && s.items.isNotEmpty(), error = null)
        viewModelScope.launch {
            try {
                val resp = if (s.q.isNotEmpty()) {
                    Api.search(
                        q = s.q,
                        limit = page,
                        offset = offset,
                        kind = s.kind,
                        source = s.source,
                        hue = s.hue,
                        tag = s.tag,
                    )
                } else {
                    Api.items(
                        offset = offset,
                        limit = page,
                        kind = s.kind,
                        source = s.source,
                        hue = s.hue,
                        tag = s.tag,
                        sort = s.sort,
                        seed = s.seed,
                    )
                }
                val newItems = Parse.cards(resp.optJSONArray("items"))
                val base = if (reset) emptyList() else grid.items
                val seen = HashSet<String>()
                val merged = (base + newItems).filter { seen.add(it.id) }
                grid = grid.copy(
                    items = merged,
                    loading = false,
                    refreshing = false,
                    hasMore = resp.optBoolean("has_more", false),
                    total = resp.optInt("total", merged.size),
                    offset = offset + page,
                    error = null,
                    authError = false,
                )
            } catch (e: ApiException) {
                grid = grid.copy(
                    loading = false,
                    refreshing = false,
                    error = e.message ?: "request failed",
                    authError = e.code == 401,
                )
            } catch (e: Exception) {
                grid = grid.copy(
                    loading = false,
                    refreshing = false,
                    error = e.message ?: "load failed",
                )
            }
        }
    }

    fun quickSave(
        text: String,
        title: String,
        file: File?,
        mime: String,
        onDone: (String) -> Unit,
        onError: (String) -> Unit,
    ) {
        viewModelScope.launch {
            try {
                val resp = if (file != null) {
                    Api.saveNoteWithFile(text, title, file, mime)
                } else {
                    Api.saveNote(text, title)
                }
                val id = resp.optString("id")
                load(reset = true)
                onDone(id)
            } catch (e: Exception) {
                onError(e.message ?: "save failed")
            }
        }
    }

    fun deleteItem(id: String, onDone: () -> Unit = {}) {
        viewModelScope.launch {
            runCatching { Api.deleteItem(id) }
            grid = grid.copy(items = grid.items.filterNot { it.id == id })
            onDone()
        }
    }

    fun refreshItem(id: String) {
        viewModelScope.launch { runCatching { Api.refreshItem(id) } }
    }

    fun explainItem(id: String) {
        viewModelScope.launch { runCatching { Api.explainItem(id) } }
    }

    fun interact(id: String) {
        viewModelScope.launch { Api.interact(id) }
    }

    // ============================================================ followups

    var followupKind by mutableStateOf("")

    fun loadFollowups(status: String = followupStatus, kind: String = followupKind) {
        followupStatus = status
        followupKind = kind
        followupsLoading = true
        viewModelScope.launch {
            try {
                val resp = Api.followups(status, kind)
                val list = Parse.followups(resp.optJSONArray("items"))
                followups = list
            } catch (_: Exception) {
            } finally {
                followupsLoading = false
            }
        }
    }

    fun updateFollowup(id: String, status: String) {
        viewModelScope.launch {
            runCatching { Api.setFollowupStatus(id, status) }
            loadFollowups(followupStatus)
        }
    }

    // ================================================================ trash

    fun loadTrash() {
        trashLoading = true
        viewModelScope.launch {
            try {
                val resp = Api.trash()
                trashItems = Parse.cards(resp.optJSONArray("items"))
                trashDays = resp.optInt("trash_days", 30)
            } catch (_: Exception) {
            } finally {
                trashLoading = false
            }
        }
    }

    fun restoreTrashItem(id: String) {
        viewModelScope.launch {
            runCatching { Api.restoreTrash(id) }
            loadTrash()
            load(reset = true)
        }
    }

    fun destroyTrashItem(id: String) {
        viewModelScope.launch {
            runCatching { Api.destroyTrash(id) }
            loadTrash()
        }
    }

    fun purgeTrash() {
        viewModelScope.launch {
            runCatching { Api.purgeTrash() }
            loadTrash()
        }
    }

    fun restoreAllTrash() {
        viewModelScope.launch {
            runCatching { Api.restoreAllTrash() }
            loadTrash()
            load(reset = true)
        }
    }

    // ============================================================== cleanup

    fun loadCleanup() {
        cleanupLoading = true
        viewModelScope.launch {
            try {
                val resp = Api.cleanupGroups()
                val groups = Parse.cleanupGroups(resp)
                cleanupGroups = groups
                cleanupScanned = resp.optInt("scanned")
                val sel = mutableSetOf<String>()
                for (g in groups) {
                    for (m in g.members) {
                        if (m.id != g.leader) sel.add(m.id)
                    }
                }
                cleanupSelected = sel
            } catch (_: Exception) {
            } finally {
                cleanupLoading = false
            }
        }
    }

    fun toggleCleanupSelection(id: String) {
        cleanupSelected = if (cleanupSelected.contains(id)) {
            cleanupSelected - id
        } else {
            cleanupSelected + id
        }
    }

    fun clearCleanupSelection() {
        cleanupSelected = emptySet()
    }

    fun cleanupDeleteSelected(onDone: (Int) -> Unit) {
        val ids = cleanupSelected.toList()
        if (ids.isEmpty()) return
        viewModelScope.launch {
            val deleted = runCatching { Api.cleanupDelete(ids).optInt("deleted") }.getOrDefault(0)
            loadCleanup()
            load(reset = true)
            onDone(deleted)
        }
    }

    fun cleanupDeleteType(channel: String, onDone: (Int) -> Unit) {
        viewModelScope.launch {
            val deleted = runCatching { Api.cleanupDeleteType(channel).optInt("deleted") }.getOrDefault(0)
            loadCleanup()
            load(reset = true)
            onDone(deleted)
        }
    }

    // =============================================================== inspire

    var brief by mutableStateOf<Brief?>(null)
        private set
    var briefLoading by mutableStateOf(false)
        private set
    var briefError by mutableStateOf<String?>(null)
        private set
    var briefAbout by mutableStateOf("")
        private set
    var briefJobId by mutableStateOf<String?>(null)
        private set

    /** Ask for a build brief. Empty about = the server picks an idea from the vault. */
    fun runBrief(about: String, seedIds: List<String> = emptyList()) {
        if (briefLoading) return
        briefAbout = about
        briefLoading = true
        briefError = null
        brief = null
        briefJobId = null
        viewModelScope.launch {
            try {
                val started = Api.inspireStart(about, seedIds)
                val jobId = started.optString("job_id")
                if (jobId.isEmpty()) throw IllegalStateException("no job id")
                briefJobId = jobId
                var tries = 0
                while (tries < 90) {
                    delay(1500)
                    tries += 1
                    val job = Api.inspireStatus(jobId)
                    when (job.optString("state")) {
                        "done" -> {
                            brief = BriefParse.fromJob(job)
                            if (brief == null) briefError = "the server returned no brief"
                            break
                        }
                        "error" -> {
                            briefError = job.optString("error").ifEmpty { "the brief failed" }
                            break
                        }
                        "missing" -> {
                            briefError = "job vanished"
                            break
                        }
                    }
                }
                if (brief == null && briefError == null) briefError = "timed out — try again"
            } catch (e: Exception) {
                briefError = e.message ?: "brief failed"
            } finally {
                briefLoading = false
            }
        }
    }

    /** Markdown of the current brief, for the copy button. */
    suspend fun briefMarkdown(): String {
        val id = briefJobId ?: return ""
        return runCatching { Api.briefMarkdown(id) }.getOrDefault("")
    }

    /** Drop the shown brief so the ask box comes back. */
    fun clearBrief() {
        brief = null
        briefError = null
        briefAbout = ""
        briefJobId = null
    }

    private fun pairs(o: JSONObject, key: String): List<Pair<String, Int>> {
        val a = o.optJSONArray(key) ?: return emptyList()
        val out = mutableListOf<Pair<String, Int>>()
        for (i in 0 until a.length()) {
            val p = a.optJSONArray(i) ?: continue
            val name = p.optString(0)
            if (name.isNotEmpty()) out.add(name to p.optInt(1))
        }
        return out
    }
}
package com.niknair.tasteengine.net

import org.json.JSONArray
import org.json.JSONObject

/** Card-level media info (the `cover` object on grid items). */
data class MediaInfo(
    val id: String = "",
    val kind: String = "",
    val file: String = "",
    val thumb: String = "",
    val width: Int = 0,
    val height: Int = 0,
    val duration: Double = 0.0,
    val mime: String = "",
    val transcript: String = "",
) {
    val best: String get() = thumb.ifEmpty { file }
    val isVideo: Boolean get() = kind == "video"

    /** Height / width ratio for layout; clamped so weird covers stay usable. */
    val aspect: Float
        get() = if (width > 0 && height > 0) {
            (height.toFloat() / width.toFloat()).coerceIn(0.55f, 1.7f)
        } else 1f
}

data class ItemCard(
    val id: String,
    val title: String,
    val kind: String,
    val source: String,
    val url: String,
    val text: String,
    val note: String,
    val author: String = "",
    val savedAt: String,
    val deletedAt: String = "",
    val cover: MediaInfo?,
    val tags: List<String>,
    val jobState: String?,
    val explanationStatus: String?,
    val openFollowups: Int,
    val similarity: Double?,
)

data class Followup(
    val id: String,
    val kind: String,
    val title: String,
    val url: String,
    val status: String,
    val why: String,
    val body: String = "",
    val itemId: String = "",
    val itemTitle: String = "",
    val itemThumb: String = "",
    val itemW: Int = 0,
    val itemH: Int = 0,
) {
    /** Same clamp as a post card, so both grids break lines the same way. */
    val aspect: Float
        get() = if (itemW > 0 && itemH > 0) {
            (itemH.toFloat() / itemW.toFloat()).coerceIn(0.55f, 1.7f)
        } else 1f
}

data class CleanupMember(
    val id: String,
    val kind: String,
    val title: String,
    val url: String,
    val source: String,
    val savedAt: String,
    val cover: MediaInfo?,
)

data class CleanupGroup(
    val leader: String?,
    val evidence: List<String>,
    val size: Int,
    val members: List<CleanupMember>,
)

/** One k-means similarity group (server `cluster` / `clusters` entry). */
data class ClusterGroup(
    val id: String = "",
    val label: String = "",
    val size: Int = 0,
    val cohesion: Double = 0.0,
    val hue: String = "",
    val kinds: Map<String, Int> = emptyMap(),
    val fittedAt: String = "",
    val covers: List<String> = emptyList(),
    /** items passing the current filter; -1 when the server did not say */
    val matched: Int = -1,
)

/** One opened group: its meta plus member cards (GET /api/groups/{id}). */
data class GroupDetail(
    val cluster: ClusterGroup,
    val items: List<ItemCard>,
)

data class ItemDetail(
    val card: ItemCard,
    val media: List<MediaInfo>,
    val explanation: JSONObject?,
    val followups: List<Followup>,
    val outlinks: List<Pair<String, String>>,
    val similar: List<ItemCard>,
)


data class FrameVaultHit(
    val id: String,
    val title: String,
    val kind: String,
    val url: String,
)

object Parse {


    fun frameVaultHits(arr: JSONArray?): List<FrameVaultHit> {
        if (arr == null) return emptyList()
        val out = ArrayList<FrameVaultHit>(arr.length())
        for (i in 0 until arr.length()) {
            val o = arr.optJSONObject(i) ?: continue
            out.add(
                FrameVaultHit(
                    id = o.optString("id"),
                    title = o.optString("title"),
                    kind = o.optString("kind"),
                    url = o.optString("url"),
                )
            )
        }
        return out
    }


    fun media(o: JSONObject?): MediaInfo? {
        if (o == null || o.isNull("id")) return null
        return MediaInfo(
            id = o.optString("id"),
            kind = o.optString("kind"),
            file = o.optString("file"),
            thumb = o.optString("thumb"),
            width = o.optInt("width"),
            height = o.optInt("height"),
            duration = o.optDouble("duration", 0.0),
            mime = o.optString("mime"),
            transcript = o.optString("transcript"),
        )
    }

    /** Cleanup member covers have no id — accept anything with a thumb/file. */
    fun mediaCover(o: JSONObject?): MediaInfo? {
        if (o == null) return null
        if (o.isNull("thumb") && o.isNull("file")) return null
        return MediaInfo(
            id = o.optString("id"),
            kind = o.optString("kind"),
            file = o.optString("file"),
            thumb = o.optString("thumb"),
            width = o.optInt("width"),
            height = o.optInt("height"),
            duration = o.optDouble("duration", 0.0),
            mime = o.optString("mime"),
            transcript = o.optString("transcript"),
        )
    }

    fun card(o: JSONObject): ItemCard {
        val tags = mutableListOf<String>()
        o.optJSONArray("tags")?.let { a ->
            for (i in 0 until a.length()) {
                val tag = a.optString(i)
                if (tag.isNotEmpty()) tags.add(tag)
            }
        }
        return ItemCard(
            id = o.optString("id"),
            title = o.optString("title"),
            kind = o.optString("kind"),
            source = o.optString("source"),
            url = o.optString("url"),
            text = o.optString("text"),
            note = o.optString("note"),
            author = o.optString("author"),
            savedAt = o.optString("saved_at"),
            deletedAt = o.optString("deleted_at"),
            cover = media(o.optJSONObject("cover")),
            tags = tags,
            jobState = o.optJSONObject("job")?.optString("state")?.ifEmpty { null },
            explanationStatus = o.optString("explanation_status").ifEmpty { null },
            openFollowups = o.optInt("open_followups"),
            similarity = if (o.has("similarity") && !o.isNull("similarity")) o.optDouble("similarity", 0.0) else null,
        )
    }

    fun cards(a: JSONArray?): List<ItemCard> {
        if (a == null) return emptyList()
        val out = mutableListOf<ItemCard>()
        for (i in 0 until a.length()) {
            a.optJSONObject(i)?.let { out.add(card(it)) }
        }
        return out
    }

    fun followup(o: JSONObject): Followup = Followup(
        id = o.optString("id"),
        kind = o.optString("kind"),
        title = o.optString("title"),
        url = o.optString("url"),
        status = o.optString("status"),
        why = o.optString("why"),
        body = o.optString("body"),
        itemId = o.optString("item_id"),
        itemTitle = o.optString("item_title"),
        itemThumb = o.optString("item_thumb"),
        itemW = o.optInt("item_w"),
        itemH = o.optInt("item_h"),
    )

    fun followups(a: JSONArray?): List<Followup> {
        if (a == null) return emptyList()
        val out = mutableListOf<Followup>()
        for (i in 0 until a.length()) {
            a.optJSONObject(i)?.let { out.add(followup(it)) }
        }
        return out
    }

    fun cleanupGroups(o: JSONObject): List<CleanupGroup> {
        val groups = mutableListOf<CleanupGroup>()
        val arr = o.optJSONArray("groups") ?: return groups
        for (i in 0 until arr.length()) {
            val g = arr.optJSONObject(i) ?: continue
            val members = mutableListOf<CleanupMember>()
            g.optJSONArray("members")?.let { ms ->
                for (j in 0 until ms.length()) {
                    val m = ms.optJSONObject(j) ?: continue
                    members.add(
                        CleanupMember(
                            id = m.optString("id"),
                            kind = m.optString("kind"),
                            title = m.optString("title"),
                            url = m.optString("url"),
                            source = m.optString("source"),
                            savedAt = m.optString("saved_at"),
                            cover = mediaCover(m.optJSONObject("cover")),
                        )
                    )
                }
            }
            val evidence = mutableListOf<String>()
            g.optJSONArray("evidence")?.let { ev ->
                for (j in 0 until ev.length()) {
                    val e = ev.optString(j)
                    if (e.isNotEmpty()) evidence.add(e)
                }
            }
            groups.add(
                CleanupGroup(
                    leader = g.optString("leader").ifEmpty { null },
                    evidence = evidence,
                    size = g.optInt("full_size"),
                    members = members,
                )
            )
        }
        return groups
    }

    /** One cluster entry from GET /api/groups — every field forgiving. */
    fun clusterGroup(o: JSONObject): ClusterGroup {
        val covers = mutableListOf<String>()
        o.optJSONArray("covers")?.let { a ->
            for (i in 0 until a.length()) {
                val c = a.optString(i)
                if (c.isNotEmpty()) covers.add(c)
            }
        }
        val kinds = mutableMapOf<String, Int>()
        o.optJSONObject("kinds")?.let { ko ->
            val it = ko.keys()
            while (it.hasNext()) {
                val k = it.next()
                kinds[k] = ko.optInt(k, 0)
            }
        }
        val matched = if (o.has("matched")) o.optInt("matched", -1) else -1
        return ClusterGroup(
            id = o.optString("id"),
            label = o.optString("label"),
            size = o.optInt("size"),
            cohesion = o.optDouble("cohesion", 0.0),
            hue = o.optString("hue"),
            kinds = kinds,
            fittedAt = o.optString("fitted_at"),
            covers = covers,
            matched = matched,
        )
    }

    fun clusterGroups(o: JSONObject): List<ClusterGroup> {
        val out = mutableListOf<ClusterGroup>()
        o.optJSONArray("clusters")?.let { a ->
            for (i in 0 until a.length()) {
                a.optJSONObject(i)?.let { out.add(clusterGroup(it)) }
            }
        }
        return out
    }

    /** GET /api/groups/{id} or /api/groups/random — cluster + member cards. */
    fun groupDetail(o: JSONObject): GroupDetail {
        val cluster = o.optJSONObject("cluster")?.let { clusterGroup(it) } ?: ClusterGroup()
        return GroupDetail(
            cluster = cluster,
            items = cards(o.optJSONArray("items")),
        )
    }

    fun detail(o: JSONObject): ItemDetail {
        val media = mutableListOf<MediaInfo>()
        o.optJSONArray("media")?.let { a ->
            for (i in 0 until a.length()) media(a.optJSONObject(i))?.let { media.add(it) }
        }
        val followups = mutableListOf<Followup>()
        o.optJSONArray("followups")?.let { a ->
            for (i in 0 until a.length()) {
                val f = a.optJSONObject(i) ?: continue
                followups.add(followup(f))
            }
        }
        val outlinks = mutableListOf<Pair<String, String>>()
        o.optJSONArray("outlinks")?.let { a ->
            for (i in 0 until a.length()) {
                val l = a.optJSONObject(i) ?: continue
                val url = l.optString("url")
                if (url.isEmpty()) continue
                val label = l.optString("title").ifEmpty { l.optString("text") }.ifEmpty { url }
                outlinks.add(label to url)
            }
        }
        return ItemDetail(
            card = card(o),
            media = media,
            explanation = o.optJSONObject("explanation"),
            followups = followups,
            outlinks = outlinks,
            similar = cards(o.optJSONArray("similar")),
        )
    }
}

// ---------------------------------------------------------------- build brief

data class BriefRef(val ref: String, val id: String, val thumb: String, val title: String)

data class BriefTheme(val name: String, val why: String)

data class BriefSwatch(val hex: String, val role: String, val name: String)

data class BriefFamily(val family: String, val role: String, val match: String, val why: String)

data class BriefPart(val name: String, val what: String)

data class BriefStep(val step: String, val detail: String)

/** One thing to make, from the saved taste. Mirrors the server brief JSON. */
data class Brief(
    val kind: String = "",
    val title: String = "",
    val oneLiner: String = "",
    val themes: List<BriefTheme> = emptyList(),
    val palette: List<BriefSwatch> = emptyList(),
    val families: List<BriefFamily> = emptyList(),
    val scaleNote: String = "",
    val shape: String = "",
    val parts: List<BriefPart> = emptyList(),
    val make: List<BriefStep> = emptyList(),
    val steal: List<String> = emptyList(),
    val buildPrompt: String = "",
    val refs: List<BriefRef> = emptyList(),
    val lensName: String = "",
)

object BriefParse {
    private fun strings(a: JSONArray?): List<String> {
        if (a == null) return emptyList()
        val out = mutableListOf<String>()
        for (i in 0 until a.length()) {
            val v = a.optString(i)
            if (v.isNotEmpty()) out.add(v)
        }
        return out
    }

    /** job = the inspire_status payload: { state, result: { brief, refs, brainstorm } } */
    fun fromJob(job: JSONObject?): Brief? {
        if (job == null) return null
        val result = job.optJSONObject("result") ?: return null
        val b = result.optJSONObject("brief") ?: return null
        val build = b.optJSONObject("build")
        val typeSystem = b.optJSONObject("type_system")

        val themes = mutableListOf<BriefTheme>()
        b.optJSONArray("themes")?.let { a ->
            for (i in 0 until a.length()) {
                val o = a.optJSONObject(i) ?: continue
                themes.add(BriefTheme(o.optString("name"), o.optString("why")))
            }
        }
        val palette = mutableListOf<BriefSwatch>()
        b.optJSONArray("palette")?.let { a ->
            for (i in 0 until a.length()) {
                val o = a.optJSONObject(i) ?: continue
                val hex = o.optString("hex")
                if (hex.isEmpty()) continue
                palette.add(BriefSwatch(hex, o.optString("role"), o.optString("name")))
            }
        }
        val families = mutableListOf<BriefFamily>()
        typeSystem?.optJSONArray("families")?.let { a ->
            for (i in 0 until a.length()) {
                val o = a.optJSONObject(i) ?: continue
                val fam = o.optString("family")
                if (fam.isEmpty()) continue
                families.add(
                    BriefFamily(
                        fam,
                        o.optString("role"),
                        o.optString("match"),
                        o.optString("why"),
                    )
                )
            }
        }
        val parts = mutableListOf<BriefPart>()
        build?.optJSONArray("parts")?.let { a ->
            for (i in 0 until a.length()) {
                val o = a.optJSONObject(i) ?: continue
                parts.add(BriefPart(o.optString("name"), o.optString("what")))
            }
        }
        val make = mutableListOf<BriefStep>()
        b.optJSONArray("make")?.let { a ->
            for (i in 0 until a.length()) {
                val o = a.optJSONObject(i) ?: continue
                make.add(BriefStep(o.optString("step"), o.optString("detail")))
            }
        }
        val refs = mutableListOf<BriefRef>()
        result.optJSONArray("refs")?.let { a ->
            for (i in 0 until a.length()) {
                val o = a.optJSONObject(i) ?: continue
                val id = o.optString("id")
                if (id.isEmpty()) continue
                val thumb = o.optString("thumb").ifEmpty { o.optJSONObject("cover")?.optString("thumb") ?: "" }
                refs.add(BriefRef(o.optString("ref"), id, thumb, o.optString("title")))
            }
        }
        return Brief(
            kind = b.optString("kind"),
            title = b.optString("title"),
            oneLiner = b.optString("one_liner"),
            themes = themes,
            palette = palette,
            families = families,
            scaleNote = typeSystem?.optString("scale_note") ?: "",
            shape = build?.optString("shape") ?: "",
            parts = parts,
            make = make,
            steal = strings(b.optJSONArray("steal")),
            buildPrompt = b.optString("build_prompt"),
            refs = refs,
            lensName = result.optJSONObject("brainstorm")?.optString("name") ?: "",
        )
    }
}

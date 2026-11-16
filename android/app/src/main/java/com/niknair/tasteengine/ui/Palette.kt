package com.niknair.tasteengine.ui

import com.niknair.tasteengine.net.Swatch
import kotlin.math.abs
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.max as maxOf
import kotlin.math.min as minOf
import kotlin.math.pow
import kotlin.math.round
import kotlin.math.sin

/**
 * Palette-style derivation — a faithful Kotlin port of web/app.js `paletteView`
 * (and its Oklab helpers), mirroring backend/src/color.rs. Pure Kotlin with no
 * Compose imports so it is unit-testable. Returns (role, hex) pairs; role is
 * null for the raw "measured" view.
 */
object Palette {

    /** The 9 palette styles, in web order; cycled by tap. */
    val STYLE_NAMES = listOf(
        "measured", "mono", "analogous", "complementary", "split-comp", "triadic",
        "roles", "dark ui", "light ui",
    )

    private data class Oklch(val l: Double, val c: Double, val h: Double)

    private data class Entry(
        val hex: String,
        val pct: Double,
        val l: Double,
        val c: Double,
        val h: Double,
    )

    private data class HarmonySlot(val role: String, val hue: Double, val l: Double, val cf: Double)

    private fun hexToRgb(hex: String): IntArray? {
        val value = hex.removePrefix("#")
        if (!Regex("^[0-9a-f]{6}$", RegexOption.IGNORE_CASE).matches(value)) return null
        return intArrayOf(
            value.substring(0, 2).toInt(16),
            value.substring(2, 4).toInt(16),
            value.substring(4, 6).toInt(16),
        )
    }

    @Suppress("unused")
    fun rgbToHsvArr(rgb: IntArray): Triple<Int, Int, Int> {
        if (rgb.size != 3) return Triple(0, 0, 0)
        val v0 = rgb[0] / 255.0
        val v1 = rgb[1] / 255.0
        val v2 = rgb[2] / 255.0
        val mx = maxOf(v0, maxOf(v1, v2))
        val mn = minOf(v0, minOf(v1, v2))
        val delta = mx - mn
        var hue = 0.0
        if (delta != 0.0) {
            when (mx) {
                v0 -> hue = 60 * (((v1 - v2) / delta) % 6)
                v1 -> hue = 60 * ((v2 - v0) / delta + 2)
                else -> hue = 60 * ((v0 - v1) / delta + 4)
            }
        }
        if (hue < 0) hue += 360
        val saturation = if (mx != 0.0) delta / mx else 0.0
        return Triple(
            round(hue).toInt(),
            round(saturation * 100).toInt(),
            round(mx * 100).toInt(),
        )
    }

    // sRGB hex -> Oklab (L, C, h) — same math as backend/src/color.rs.
    private fun hexToOklch(hex: String): Oklch? {
        val rgb = hexToRgb(hex) ?: return null
        val lin = DoubleArray(3)
        for (i in 0 until 3) {
            val c = rgb[i] / 255.0
            lin[i] = if (c <= 0.04045) c / 12.92 else ((c + 0.055) / 1.055).pow(2.4)
        }
        val r = lin[0]
        val g = lin[1]
        val b = lin[2]
        val l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b
        val m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b
        val s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b
        val lRoot = l.cbrt()
        val mRoot = m.cbrt()
        val sRoot = s.cbrt()
        val a = 1.9779984951 * lRoot - 2.428592205 * mRoot + 0.4505937099 * sRoot
        val b2 = 0.0259040371 * lRoot + 0.7827717662 * mRoot - 0.808675766 * sRoot
        return Oklch(
            l = 0.2104542553 * lRoot + 0.793617785 * mRoot - 0.0040720468 * sRoot,
            c = hypot(a, b2),
            h = (atan2(b2, a) * 180 / Math.PI + 360) % 360,
        )
    }

    private fun Double.cbrt(): Double = if (this >= 0) this.pow(1.0 / 3.0) else -((-this).pow(1.0 / 3.0))

    // Oklab -> sRGB hex, re-targeting L. Shrinks chroma until in gamut; falls
    // back to a mid gray.
    private fun oklchToHex(lab: Oklch, targetL: Double): String {
        var c = lab.c
        val h = lab.h
        val rad = (h * Math.PI) / 180
        for (i in 0 until 24) {
            val a = c * cos(rad)
            val b = c * sin(rad)
            val lRoot = targetL + 0.3963377774 * a + 0.2158037573 * b
            val mRoot = targetL - 0.1055613458 * a - 0.0638541728 * b
            val sRoot = targetL - 0.0894841775 * a - 1.291485548 * b
            val l2 = lRoot.pow(3)
            val m2 = mRoot.pow(3)
            val s2 = sRoot.pow(3)
            val r = 4.0767416621 * l2 - 3.3077115913 * m2 + 0.2309699292 * s2
            val g = -1.2684380046 * l2 + 2.6097574011 * m2 - 0.3413193965 * s2
            val b2 = -0.0041960863 * l2 - 0.7034186147 * m2 + 1.707614701 * s2
            if (r >= -0.002 && r <= 1.002 && g >= -0.002 && g <= 1.002 && b2 >= -0.002 && b2 <= 1.002) {
                val unlin = { v: Double ->
                    val t = if (v <= 0.0031308) 12.92 * v else 1.055 * maxOf(v, 0.0).pow(1 / 2.4) - 0.055
                    round(minOf(255.0, maxOf(0.0, t * 255))).toInt()
                }
                return "#" + listOf(unlin(r), unlin(g), unlin(b2))
                    .joinToString("") { it.toString(16).padStart(2, '0') }
            }
            c *= 0.92
            if (c < 0.004) {
                val gray = round(minOf(255.0, maxOf(0.0, targetL * 255))).toInt()
                return "#" + listOf(gray, gray, gray)
                    .joinToString("") { it.toString(16).padStart(2, '0') }
            }
        }
        return "#808080"
    }

    private fun mod(a: Double): Double = ((a % 360) + 360) % 360

    private fun harmonySlots(styleName: String, h: Double): List<HarmonySlot> = when (styleName) {
        "mono" -> listOf(
            HarmonySlot("ink", mod(h), 0.16, 0.55),
            HarmonySlot("deep", mod(h), 0.32, 0.9),
            HarmonySlot("mid", mod(h), 0.52, 1.0),
            HarmonySlot("soft", mod(h), 0.72, 0.75),
            HarmonySlot("tint", mod(h), 0.88, 0.4),
        )
        "analogous" -> listOf(
            HarmonySlot("-30", mod(h + 330), 0.42, 0.9),
            HarmonySlot("-15", mod(h + 345), 0.6, 1.0),
            HarmonySlot("base", h, 0.5, 1.0),
            HarmonySlot("+15", mod(h + 15), 0.68, 0.9),
            HarmonySlot("+30", mod(h + 30), 0.34, 0.9),
        )
        "complementary" -> listOf(
            HarmonySlot("base", mod(h), 0.32, 0.9),
            HarmonySlot("base lt", mod(h), 0.6, 1.0),
            HarmonySlot("opposite", mod(h + 180), 0.32, 0.9),
            HarmonySlot("opposite lt", mod(h + 180), 0.6, 1.0),
            HarmonySlot("opposite tint", mod(h + 180), 0.82, 0.5),
        )
        "split-comp" -> listOf(
            HarmonySlot("base", mod(h), 0.3, 0.9),
            HarmonySlot("base lt", mod(h), 0.58, 1.0),
            HarmonySlot("+150", mod(h + 150), 0.55, 0.95),
            HarmonySlot("+210", mod(h + 210), 0.55, 0.95),
            HarmonySlot("+210 lt", mod(h + 210), 0.8, 0.5),
        )
        "triadic" -> listOf(
            HarmonySlot("base", mod(h), 0.3, 0.9),
            HarmonySlot("base lt", mod(h), 0.58, 1.0),
            HarmonySlot("+120", mod(h + 120), 0.55, 0.95),
            HarmonySlot("+240", mod(h + 240), 0.55, 0.95),
            HarmonySlot("+240 lt", mod(h + 240), 0.8, 0.5),
        )
        else -> emptyList()
    }

    private fun hueDist(a: Double, b: Double): Double {
        val d = abs(a - b)
        return minOf(d, 360 - d)
    }

    /**
     * Derived framings of the measured palette. Measured shows the share-ordered
     * swatches as extracted; roles/harmony/ui styles pick and retarget from the
     * same colors. Returns (role, hex) pairs in display order.
     */
    fun paletteView(palette: List<Swatch>, styleName: String): List<Pair<String?, String>> {
        val entries = palette.mapNotNull { p ->
            val hex = p.hex.lowercase()
            if (!Regex("^#[0-9a-f]{6}$").matches(hex)) return@mapNotNull null
            val ok = hexToOklch(hex) ?: return@mapNotNull null
            Entry(hex, p.pct, ok.l, ok.c, ok.h)
        }
        if (entries.isEmpty()) return emptyList()
        if (styleName == "measured") return entries.map { null to it.hex }

        val chromatic = entries.filter { it.c >= 0.06 }
        val byShare = chromatic.sortedByDescending { it.pct }
        val byVivid = chromatic.sortedByDescending { it.c }
        val byLight = entries.sortedByDescending { it.l }
        val byDark = entries.sortedBy { it.l }

        if (styleName == "roles") {
            val out = mutableListOf<Pair<String?, String>>()
            fun take(role: String, e: Entry?) {
                if (e != null && out.none { it.second == e.hex }) out.add(role to e.hex)
            }
            val primary = byShare.firstOrNull()
            take("primary", primary)
            val secondary = byShare.find { e ->
                out.none { it.second == e.hex } &&
                    (out.isEmpty() || (primary != null && hueDist(e.h, primary.h) >= 45))
            } ?: byShare.getOrNull(1)
            take("secondary", secondary)
            take("accent", byVivid.find { e -> out.none { it.second == e.hex } })
            take("ink", byDark.firstOrNull())
            val surface = byLight.find { e -> out.none { it.second == e.hex } } ?: byLight.firstOrNull()
            take("surface", surface)
            return out
        }

        if (harmonySlots(styleName, 0.0).isNotEmpty()) {
            val anchor = byShare.firstOrNull() ?: entries.sortedByDescending { it.c }.firstOrNull()
            if (anchor == null) return emptyList()
            val c = minOf(maxOf(anchor.c, 0.03), 0.25)
            return harmonySlots(styleName, anchor.h).map { slot ->
                val reused = chromatic.find { e ->
                    hueDist(e.h, slot.hue) <= 18 && abs(e.l - slot.l) <= 0.15
                }
                val hex = if (reused != null) reused.hex
                else oklchToHex(Oklch(slot.l, c * slot.cf, slot.hue), slot.l)
                slot.role to hex
            }
        }

        // dark ui / light ui: an interface theme read out of the image.
        val dark = styleName == "dark ui"
        val out = mutableListOf<Pair<String?, String>>()
        fun push(role: String, hex: String?) {
            if (hex != null && out.none { it.second == hex }) out.add(role to hex)
        }
        push("bg", if (dark) byDark[0].hex else oklchToHex(Oklch(byLight[0].l, byLight[0].c, byLight[0].h), maxOf(byLight[0].l, 0.92)))
        if (out.isNotEmpty() && (dark && byDark.size > 1 || !dark && byLight.size > 1)) {
            push("surface", if (dark) byDark[1].hex else oklchToHex(Oklch(byLight[1].l, byLight[1].c, byLight[1].h), maxOf(byLight[1].l, 0.85)))
        }
        push("text", if (dark) oklchToHex(Oklch(byLight[0].l, byLight[0].c, byLight[0].h), maxOf(byLight[0].l, 0.88))
        else oklchToHex(Oklch(byDark[0].l, byDark[0].c, byDark[0].h), minOf(byDark[0].l, 0.22)))
        val accent = byVivid.firstOrNull()
        push("accent", accent?.hex)
        if (accent != null) {
            val second = byVivid.find { e -> e.hex != accent.hex && hueDist(e.h, accent.h) >= 60 }
            push("accent-2", second?.hex)
        }
        return out
    }
}
//! One-to-one HTML export of a finished inspire run — port of inspire_export.py.

use std::io::Read;

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::mediaio;
use crate::vault;

const PAGE_CSS: &str = r#"
:root { color-scheme: dark; }
* { box-sizing: border-box; border-radius: 0 !important; }
body { margin: 0; background: #151110; color: #f2eefa;
       font-family: system-ui, -apple-system, 'Segoe UI', sans-serif; line-height: 1.5; }
main { max-width: 1080px; margin: 0 auto; padding: 40px 28px 80px; }
h1 { color: #e4628b; font-size: 1.7rem; margin: 0 0 4px; letter-spacing: .02em; }
h2 { color: #e4628b; font-size: .82rem; text-transform: uppercase;
     letter-spacing: .16em; margin: 34px 0 10px; font-weight: 600; }
.sub { color: #a49385; font-size: .84rem; margin: 4px 0 12px; }
.meta { color: #a49385; font-size: .74rem; margin: 0 0 6px; }
.summary { color: #f2eefa; max-width: 74ch; line-height: 1.55; margin: 8px 0 0; }
.tags { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 12px; }
.tags span { border: 1px solid #3a322f; padding: 4px 11px; font-size: .76rem; color: #d8c9c2; }
.swatches { display: flex; flex-wrap: wrap; gap: 10px; }
.swatch { width: 118px; height: 86px; padding: 9px 10px; display: flex;
          flex-direction: column; justify-content: flex-end; gap: 2px;
          border: 1px solid #3a322f; }
.swatch b { font-size: .78rem; }
.swatch i { font-size: .68rem; opacity: .8; font-style: normal; }
.swatch code { font-size: .66rem; opacity: .85; font-variant-numeric: tabular-nums; }
.families { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 12px; }
.family { border: 1px solid #3a322f; padding: 14px; }
.fam-name { font-size: 1.3rem; font-weight: 700; display: block; }
.frole { font-size: .66rem; text-transform: uppercase; letter-spacing: .12em; color: #a49385; }
.family p { color: #a49385; font-size: .82rem; margin: 8px 0 0; line-height: 1.45; }
.material { border: 1px solid #3a322f; background: #1d1815; padding: 14px; }
.mname { color: #e4628b; font-weight: 700; letter-spacing: .08em;
         text-transform: uppercase; font-size: .9rem; }
.concept { margin: 6px 0 12px; }
.facets { list-style: none; margin: 0; padding: 0; display: grid;
          grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 10px; }
.facets li { border: 1px solid #3a322f; padding: 12px 14px; background: #1a1514; }
.facets strong { display: block; }
.facets p { color: #a49385; font-size: .82rem; margin: 4px 0 6px; }
.frefs { color: #7d6f68; font-size: .7rem; }
.shots, .refs { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
                gap: 12px; }
.shot, .ref { margin: 0; border: 1px solid #3a322f; background: #1d1815; }
.shot img, .ref img { width: 100%; aspect-ratio: 1; object-fit: cover; display: block; }
.imgmiss { aspect-ratio: 1; display: flex; align-items: center; justify-content: center;
           color: #7d6f68; font-size: .8rem; }
figcaption { padding: 8px 10px; font-size: .74rem; color: #d8c9c2; display: flex;
             flex-direction: column; gap: 2px; }
figcaption span { color: #a49385; }
footer { margin-top: 36px; color: #7d6f68; font-size: .74rem;
         border-top: 1px solid #3a322f; padding-top: 12px; }
a { color: #e4628b; }
.note { color: #a49385; font-size: .78rem; margin: 3px 0; }
"#;

fn esc(s: impl std::fmt::Display) -> String {
    let s = s.to_string();
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

fn media_data_uri(name: &str, max_edge: u32) -> String {
    if name.is_empty() {
        return String::new();
    }
    let base = mediaio::media_dir()
        .join(std::path::Path::new(name).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default());
    if !base.exists() {
        return String::new();
    }
    let Ok(data) = std::fs::read(&base) else { return String::new() };
    let Ok(img) = image::load_from_memory(&data) else { return String::new() };
    let rgb0 = img.to_rgb8();
    let (w, h) = (rgb0.width(), rgb0.height());
    let final_img = if w.max(h) > max_edge {
        let ratio = max_edge as f64 / w.max(h) as f64;
        let nw = ((w as f64 * ratio) as u32).max(1);
        let nh = ((h as f64 * ratio) as u32).max(1);
        image::DynamicImage::ImageRgb8(image::imageops::resize(
            &rgb0, nw, nh, image::imageops::FilterType::Lanczos3,
        ))
    } else {
        image::DynamicImage::ImageRgb8(rgb0)
    };
    let mut out: Vec<u8> = Vec::new();
    {
        let rgb8 = final_img.to_rgb8();
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82);
        if enc.encode(&rgb8, rgb8.width(), rgb8.height(), image::ExtendedColorType::Rgb8).is_err() {
            return String::new();
        }
    }
    use base64::Engine;
    format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&out)
    )
}

fn ink(hex_value: &str) -> String {
    let value = hex_value.trim_start_matches('#');
    if value.len() != 6 {
        return "#f2eefa".into();
    }
    let mut rgb = [0f64; 3];
    for (i, idx) in [0usize, 2usize, 4usize].iter().enumerate() {
        rgb[i] = u8::from_str_radix(&value[*idx..*idx + 2], 16).unwrap_or(0) as f64 / 255.0;
    }
    let lin: Vec<f64> = rgb
        .iter()
        .map(|c| if *c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) })
        .collect();
    let luminance = 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
    if luminance > 0.33 { "#17121f".into() } else { "#f2eefa".into() }
}

fn font_link(family: &str) -> String {
    let re = regex::Regex::new(r"[^A-Za-z0-9 ]").unwrap();
    let fam = re.replace_all(family, "").trim().to_string();
    if fam.is_empty() {
        return String::new();
    }
    format!(
        "<link rel=\"stylesheet\" href=\"https://fonts.googleapis.com/css2?family={}:wght@400;600;800&display=swap\">",
        fam.replace(' ', "+")
    )
}

pub fn safe_filename(name: &str) -> String {
    let base = regex::Regex::new(r"[^a-z0-9]+")
        .unwrap()
        .replace_all(&name.to_lowercase(), "-")
        .trim_matches('-')
        .to_string();
    let base = if base.is_empty() { "direction".to_string() } else { base };
    base.chars().take(40).collect()
}

/// (html, filename) for a finished inspire job
pub fn build_export_html(conn: &Connection, job_id: &str) -> Result<(String, String), String> {
    let row = vault::fetch_rows(conn, "SELECT result, created_at FROM inspire_jobs WHERE job_id=?", &[json!(job_id)])
        .into_iter()
        .next()
        .ok_or_else(|| "no finished inspire run with that id".to_string())?;
    let result_raw = row.get("result").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if result_raw.is_empty() {
        return Err("no finished inspire run with that id".into());
    }
    let result: Value = serde_json::from_str(&result_raw).map_err(|_| "stored inspire result is unreadable".to_string())?;
    let direction = result.get("direction").cloned().unwrap_or(json!({}));
    let candidates = result.get("candidates").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let guidance = result.get("palette_guidance").cloned().unwrap_or(json!({}));
    let created = row.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
    let generated: String = created.chars().take(19).collect();

    let palette = direction.get("palette").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let typo = direction.get("typography").cloned().unwrap_or(json!({}));
    let families = typo.get("families").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let materials = direction.get("materials").cloned().unwrap_or(json!({}));
    let facets = materials.get("facets").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let tags = direction.get("tags").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let title = direction.get("title").and_then(|v| v.as_str()).unwrap_or("direction").to_string();

    // preview shots
    let mut shots_html = String::new();
    let prev = vault::fetch_rows(
        conn,
        "SELECT result FROM preview_jobs WHERE inspire_job_id=? AND state='done' ORDER BY rowid DESC LIMIT 1",
        &[json!(job_id)],
    )
    .into_iter()
    .next();
    if let Some(prev) = prev {
        if let Ok(pdata) = serde_json::from_str::<Value>(prev.get("result").and_then(|v| v.as_str()).unwrap_or("{}")) {
            let mut figures = Vec::new();
            for (i, name) in pdata.get("images").and_then(|v| v.as_array()).map(|a| a.to_vec()).unwrap_or_default().into_iter().enumerate() {
                let uri = media_data_uri(name.as_str().unwrap_or(""), 512);
                if !uri.is_empty() {
                    figures.push(format!(
                        "<figure class=\"shot\"><img src=\"{uri}\" alt=\"preview {}\"><figcaption>variant {}</figcaption></figure>",
                        i + 1,
                        i + 1
                    ));
                }
            }
            if !figures.is_empty() {
                let subj = if pdata.get("subject").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false) {
                    format!("<p class=\"sub\">subject: {}</p>", esc(pdata.get("subject").and_then(|v| v.as_str()).unwrap_or("")))
                } else {
                    String::new()
                };
                shots_html = format!(
                    "<section><h2>previews</h2>{subj}<div class=\"shots\">{}</div></section>",
                    figures.join("")
                );
            }
        }
    }

    // vault moodboard
    let mut ref_figs = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        let uri = media_data_uri(c.get("thumb").and_then(|v| v.as_str()).unwrap_or(""), 512);
        let visual = if uri.is_empty() {
            "<div class=\"imgmiss\">no media</div>".to_string()
        } else {
            format!("<img src=\"{uri}\" alt=\"reference {i}\" loading=\"lazy\">")
        };
        let url = c.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let link = if url.starts_with("http") {
            format!(" · <a href=\"{}\">source</a>", esc(url))
        } else {
            String::new()
        };
        let style_label = c.get("style_label").and_then(|v| v.as_str()).unwrap_or("");
        let style = c.get("style").and_then(|v| v.as_str()).unwrap_or("");
        let style_disp = if !style_label.is_empty() { style_label } else { style };
        let sim = c.get("similarity").map(|v| v.to_string()).unwrap_or_default();
        ref_figs.push(format!(
            "<figure class=\"ref\">{visual}<figcaption><strong>{}</strong><span>{} · sim {}{}</span></figcaption></figure>",
            esc(c.get("title").and_then(|v| v.as_str()).unwrap_or("(untitled)")),
            esc(style_disp),
            esc(sim),
            link
        ));
    }
    let refs_html = if !ref_figs.is_empty() {
        format!(
            "<section><h2>drawn from your vault — {} refs at similarity &ge; {}</h2><div class=\"refs\">{}</div></section>",
            candidates.len(),
            esc(result.get("threshold").map(|v| v.to_string()).unwrap_or_else(|| "0".into())),
            ref_figs.join("")
        )
    } else {
        String::new()
    };

    // palette / typography / materials
    let swatches: String = palette
        .iter()
        .map(|p| {
            let hex = p.get("hex").and_then(|v| v.as_str()).unwrap_or("");
            format!(
                "<div class=\"swatch\" style=\"background:{};color:{}\"><b>{}</b><i>{}</i><code>{}</code></div>",
                esc(hex),
                ink(hex),
                esc(p.get("name").and_then(|v| v.as_str()).unwrap_or("")),
                esc(p.get("role").and_then(|v| v.as_str()).unwrap_or("")),
                esc(hex.to_lowercase())
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let family_cards: String = families
        .iter()
        .map(|f| {
            format!(
                "<div class=\"family\" style=\"font-family:'{}',system-ui,sans-serif\"><span class=\"fam-name\">{}</span><span class=\"frole\">{}</span><p>{}</p></div>",
                esc(f.get("family").and_then(|v| v.as_str()).unwrap_or("")),
                esc(f.get("family").and_then(|v| v.as_str()).unwrap_or("")),
                esc(f.get("role").and_then(|v| v.as_str()).unwrap_or("body")),
                esc(f.get("why").and_then(|v| v.as_str()).unwrap_or(""))
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let facet_items: String = facets
        .iter()
        .map(|f| {
            let refs: Vec<String> = f
                .get("refs")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|t| t.as_str()).map(|s| s.to_string()).collect())
                .unwrap_or_default();
            format!(
                "<li><strong>{}</strong><p>{}</p><span class=\"frefs\">{}</span></li>",
                esc(f.get("name").and_then(|v| v.as_str()).unwrap_or("")),
                esc(f.get("what").and_then(|v| v.as_str()).unwrap_or("")),
                esc(refs.join(", "))
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let tag_chips: String = tags.iter().map(|t| format!("<span>{}</span>", esc(t.as_str().unwrap_or("")))).collect::<Vec<_>>().join("");
    let warnings: Vec<String> = guidance
        .get("warnings")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|w| w.as_str()).map(|s| s.to_string()).collect())
        .unwrap_or_default();
    let warning_html = if !warnings.is_empty() {
        format!("<p class=\"note\">palette note: {}</p>", esc(warnings.join(" ")))
    } else {
        String::new()
    };
    let font_links: String = families
        .iter()
        .map(|f| font_link(f.get("family").and_then(|v| v.as_str()).unwrap_or("")))
        .collect::<Vec<_>>()
        .join("");
    let typo_direction = typo.get("direction").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let summary = direction.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mname = materials.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mconcept = materials.get("concept").and_then(|v| v.as_str()).unwrap_or("").to_string();

    let mut doc = String::new();
    doc.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    doc.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    doc.push_str(&format!("<title>taste engine — {}</title>\n", esc(&title)));
    doc.push_str(&font_links);
    doc.push_str(&format!("<style>{PAGE_CSS}</style>\n</head>\n<body>\n<main>\n"));
    doc.push_str("<header>");
    doc.push_str("<p class=\"meta\">taste engine · inspire export");
    if !generated.is_empty() {
        doc.push_str(&format!(" · {}", esc(&generated)));
    }
    doc.push_str(&format!(" · job {}</p>", esc(job_id)));
    doc.push_str(&format!("<h1>{}</h1>", esc(&title)));
    for w in &warnings {
        doc.push_str(&format!("<p class=\"note\">palette note: {}</p>", esc(w)));
    }
    doc.push_str("</header>");
    if !summary.is_empty() {
        doc.push_str(&format!("<p class=\"summary\">{}</p>", esc(&summary)));
    }
    if !tag_chips.is_empty() {
        doc.push_str(&format!("<div class=\"tags\">{tag_chips}</div>"));
    }
    if !swatches.is_empty() {
        doc.push_str(&format!("<section><h2>palette</h2><div class=\"swatches\">{swatches}</div></section>"));
    }
    if !family_cards.is_empty() {
        doc.push_str("<section><h2>typography</h2>");
        if !typo_direction.is_empty() {
            doc.push_str(&format!("<p class=\"sub\">{}</p>", esc(&typo_direction)));
        }
        doc.push_str(&format!("<div class=\"families\">{family_cards}</div></section>"));
    }
    if !mname.is_empty() || !mconcept.is_empty() || !facet_items.is_empty() {
        doc.push_str("<section><h2>materials · one language</h2><div class=\"material\">");
        doc.push_str(&format!("<span class=\"mname\">{}</span>", esc(&mname)));
        if !mconcept.is_empty() {
            doc.push_str(&format!("<p class=\"concept\">{}</p>", esc(&mconcept)));
        }
        doc.push_str(&format!("<ul class=\"facets\">{facet_items}</ul></div></section>"));
    }
    doc.push_str(&shots_html);
    doc.push_str(&refs_html);
    doc.push_str(&format!(
        "<footer>one-to-one export of inspire job {} · {} vault refs · images inlined as data URIs</footer>",
        esc(job_id),
        candidates.len()
    ));
    doc.push_str("</main>\n</body>\n</html>");

    let filename = format!("inspire-{}-{job_id}.html", safe_filename(&title));
    Ok((doc, filename))
}

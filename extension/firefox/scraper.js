// Taste Engine scraper — injected into a platform page (same-origin context).
// Runs the platform's own internal APIs from the page's origin, so the user's
// logged-in session cookies authorize every call. No CORS, no cookie API.
// Responds to {action: "tv-scrape", task} with {ok, items} or {ok:false, error}.
"use strict";

// Guard: the page may be injected more than once. Keep one listener per document.
if (window.__tvScraperInstalled) {
  // still announce readiness so a re-injection doesn't deadlock the sender
} else {
  window.__tvScraperInstalled = true;

  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  function platformOfHost() {
    const h = location.hostname.toLowerCase();
    if (h.includes("x.com") || h.includes("twitter.com")) return "twitter";
    if (h.includes("reddit")) return "reddit";
    if (h.includes("instagram")) return "instagram";
    if (h.includes("pinterest")) return "pinterest";
    return null;
  }

  // ---------------------------------------------------------------- X / Twitter
  const X_BASE = "/i/api/graphql";
  const X_BEARER = "Bearer AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";
  const QUERY_IDS = { Bookmarks: "aqjes8lRHRFG0HUglVTfNg" };

  const X_FEATURE_NAMES = [
    "rweb_video_screen_enabled", "rweb_cashtags_enabled",
    "profile_label_improvements_pcf_label_in_post_enabled",
    "responsive_web_profile_redirect_enabled", "rweb_tipjar_consumption_enabled",
    "verified_phone_label_enabled", "creator_subscriptions_tweet_preview_api_enabled",
    "responsive_web_graphql_timeline_navigation_enabled",
    "premium_content_api_read_enabled",
    "communities_web_enable_tweet_community_results_fetch",
    "c9s_tweet_anatomy_moderator_badge_enabled",
    "responsive_web_grok_analyze_button_fetch_trends_enabled",
    "responsive_web_grok_analyze_post_followups_enabled",
    "rweb_cashtags_composer_attachment_enabled", "responsive_web_jetfuel_frame",
    "responsive_web_grok_share_attachment_enabled",
    "responsive_web_grok_annotations_enabled", "articles_preview_enabled",
    "responsive_web_edit_tweet_api_enabled",
    "rweb_conversational_replies_downvote_enabled",
    "graphql_is_translatable_rweb_tweet_is_translatable_enabled",
    "view_counts_everywhere_api_enabled",
    "longform_notetweets_consumption_enabled",
    "responsive_web_twitter_article_tweet_consumption_enabled",
    "content_disclosure_indicator_enabled",
    "content_disclosure_ai_generated_indicator_enabled",
    "responsive_web_grok_show_grok_translated_post",
    "responsive_web_grok_analysis_button_from_backend",
    "post_ctas_fetch_enabled", "freedom_of_speech_not_reach_fetch_enabled",
    "standardized_nudges_misinfo",
    "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled",
    "longform_notetweets_rich_text_read_enabled",
    "longform_notetweets_inline_media_enabled",
    "responsive_web_grok_image_annotation_enabled",
    "responsive_web_grok_imagine_annotation_enabled",
    "responsive_web_grok_community_note_auto_translation_is_enabled",
    "responsive_web_enhance_cards_enabled",
  ];
  const X_FEATURES = Object.fromEntries(X_FEATURE_NAMES.map((n) => [n, false]));
  Object.assign(X_FEATURES, {
    responsive_web_graphql_timeline_navigation_enabled: true,
    responsive_web_graphql_skip_user_profile_image_extensions_enabled: false,
    verified_phone_label_enabled: false,
    creator_subscriptions_tweet_preview_api_enabled: true,
    c9s_tweet_anatomy_moderator_badge_enabled: true,
    tweetypie_unmention_optimization_enabled: true,
    responsive_web_edit_tweet_api_enabled: true,
    graphql_is_translatable_rweb_tweet_is_translatable_enabled: true,
    view_counts_everywhere_api_enabled: true,
    longform_notetweets_consumption_enabled: true,
    responsive_web_twitter_article_tweet_consumption_enabled: true,
    freedom_of_speech_not_reach_fetch_enabled: true,
    standardized_nudges_misinfo: true,
    tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled: true,
    longform_notetweets_rich_text_read_enabled: true,
    longform_notetweets_inline_media_enabled: true,
    responsive_web_enhance_cards_enabled: false,
  });
  const X_FIELD_TOGGLES = {
    withPayments: true, withAuxiliaryUserLabels: true,
    withArticleRichContentState: true, withArticlePlainText: true,
    withArticleSummaryText: true, withArticleVoiceOver: true,
    withGrokAnalyze: true, withDisallowedReplyControls: true,
  };

  function xCookie(name) {
    const m = document.cookie.match(new RegExp("(?:^|; )" + name + "=([^;]*)"));
    return m ? decodeURIComponent(m[1]) : "";
  }

  async function xGraphql(operationName, variables) {
    const queryId = QUERY_IDS[operationName];
    const csrfToken = xCookie("ct0");
    if (!csrfToken) throw new Error("X ct0 cookie is unavailable — log in to X in this browser");
    const url = new URL(`${X_BASE}/${queryId}/${operationName}`, location.href);
    url.searchParams.set("variables", JSON.stringify(variables));
    url.searchParams.set("features", JSON.stringify(X_FEATURES));
    url.searchParams.set("fieldToggles", JSON.stringify(X_FIELD_TOGGLES));

    const response = await fetch(url.toString(), {
      credentials: "same-origin",
      referrer: location.href,
      referrerPolicy: "no-referrer-when-downgrade",
      headers: {
        "Accept": "application/json",
        "Authorization": X_BEARER,
        "X-Csrf-Token": csrfToken,
        "X-Twitter-Active-User": "yes",
        "X-Twitter-Auth-Type": "OAuth2Session",
        "X-Twitter-Client-Language": "en",
      },
    });
    const text = await response.text();
    if (!response.ok) {
      if (response.status === 429) throw new Error("X rate limited the export");
      throw new Error("X " + response.status + ": " + text.substring(0, 220));
    }
    let json;
    try { json = JSON.parse(text); } catch (e) { throw new Error("X returned non-JSON: " + text.substring(0, 160)); }
    if (json.errors && json.errors.length) {
      throw new Error("X GraphQL: " + json.errors.map((e) => e.message).join("; "));
    }
    return json;
  }

  function xUnwrapTweet(result) {
    let cur = result;
    for (let i = 0; i < 4 && cur; i++) {
      if (cur.__typename === "Tweet" || cur.legacy) return cur;
      cur = cur.tweet || cur.result || (cur.tweet_results && cur.tweet_results.result);
    }
    return cur;
  }

  function xNoteText(tweet) {
    const note = tweet && tweet.note_tweet && tweet.note_tweet.note_tweet_results && tweet.note_tweet.note_tweet_results.result;
    return (note && note.text) || "";
  }

  function xPhotoUrls(media) {
    // Keep every photo in the tweet; each CDN URL gets ?name=orig for max size.
    return [...new Set(
      (media || []).filter((i) => i.type === "photo").map((i) => i.media_url_https).filter(Boolean)
        .map((u) => (u.includes("?") ? u.replace(/name=[^&]+/, "name=orig") : u + "?format=jpg&name=orig"))
    )];
  }

  function xExtractTweet(entry) {
    const raw = entry && entry.content && entry.content.itemContent && entry.content.itemContent.tweet_results && entry.content.itemContent.tweet_results.result;
    const tweet = xUnwrapTweet(raw);
    if (!tweet || !tweet.legacy) return null;
    const legacy = tweet.legacy;
    const userResult = tweet.core && tweet.core.user_results && tweet.core.user_results.result;
    const user = (userResult && userResult.legacy) || (userResult && userResult.core) || {};
    const username = user.screen_name || (userResult && userResult.screen_name) || "";
    const tweetId = legacy.id_str || tweet.rest_id || (raw && raw.rest_id) || "";
    const text = xNoteText(tweet) || legacy.full_text || "";
    const canonicalUrl = username && tweetId ? `https://x.com/${username}/status/${tweetId}` : (tweetId ? `https://x.com/i/status/${tweetId}` : "");
    const uniqImages = xPhotoUrls((legacy.extended_entities && legacy.extended_entities.media) || (legacy.entities && legacy.entities.media) || []);
    return {
      url: canonicalUrl,
      title: text.replace(/\s+/g, " ").trim().substring(0, 160) || `Post by ${username}`,
      description: text,
      tags: ["twitter", "x", username].filter(Boolean).join(", "),
      image_url: uniqImages[0] || "",
      image_urls: uniqImages,
      platform: "twitter",
      tweet_id: tweetId,
    };
  }

  function xBottomCursor(entries) {
    const bottom = entries.find((e) => e && e.content && e.content.entryType === "TimelineTimelineCursor" && e.content.cursorType === "Bottom");
    return {
      value: bottom && bottom.content && bottom.content.value,
      stopOnEmpty: !!(bottom && bottom.content && bottom.content.stopOnEmptyResponse),
    };
  }

  async function xExportBookmarks() {
    const results = [];
    const seen = new Set();
    let cursor;
    while (true) {
      const variables = { includePromotedContent: true, count: 100 };
      if (cursor) variables.cursor = cursor;
      const json = await xGraphql("Bookmarks", variables);
      const timeline = json && json.data && json.data.bookmark_timeline_v2 && json.data.bookmark_timeline_v2.timeline;
      if (!timeline) {
        const keys = Object.keys((json && json.data) || {}).join(", ") || "none";
        throw new Error("X bookmark timeline is missing. Response keys: " + keys);
      }
      const instruction = (timeline.instructions || []).find((i) => i.type === "TimelineAddEntries");
      const entries = (instruction && instruction.entries) || [];
      let pageCount = 0;
      for (const entry of entries) {
        if (entry && entry.content && entry.content.entryType !== "TimelineTimelineItem") continue;
        const tweet = xExtractTweet(entry);
        if (!tweet || !tweet.tweet_id || seen.has(tweet.tweet_id)) continue;
        seen.add(tweet.tweet_id);
        results.push(tweet);
        pageCount++;
      }
      const cursorInfo = xBottomCursor(entries);
      if (!cursorInfo.value || (cursorInfo.stopOnEmpty && pageCount === 0)) break;
      cursor = cursorInfo.value;
    }
    return results;
  }

  // ---------------------------------------------------------------- Reddit
  function redditItemFromChild(child) {
    const data = child.data;
    const isPost = child.kind === "t3";
    const permalink = data.permalink ? `https://www.reddit.com${data.permalink}` : "";
    const title = (isPost ? data.title : data.link_title || data.body || "").replace(/\s+/g, " ").trim().substring(0, 160);
    const description = isPost ? (data.selftext || data.title || "") : (data.body || data.link_title || "");
    let imageUrls = [];
    const unescapeReddit = (u) => u.replace(/&amp;/g, "&");
    // Reddit galleries: map media_id -> i.redd.it/xxx URL (data.media_metadata).
    const galleryItems = data.gallery_data && data.gallery_data.items;
    if (Array.isArray(galleryItems) && data.media_metadata) {
      for (const g of galleryItems) {
        const meta = data.media_metadata[g && g.media_id];
        const u = meta && meta.s && meta.s.u;
        if (u) imageUrls.push(unescapeReddit(u));
      }
    }
    if (!imageUrls.length) {
      imageUrls = ((data.preview && data.preview.images) || [])
        .map((p) => p && p.source && p.source.url)
        .filter(Boolean)
        .map(unescapeReddit);
    }
    if (!imageUrls.length && data.thumbnail && data.thumbnail.startsWith("http")) imageUrls = [data.thumbnail];
    if (!imageUrls.length && data.url && /\.(?:jpe?g|png|gif|webp)(?:\?|$)/i.test(data.url)) imageUrls = [data.url];
    const imageUrl = imageUrls[0] || "";
    return {
      url: permalink,
      title: title || "Reddit post",
      description,
      tags: ["reddit", data.subreddit ? `r/${data.subreddit}` : "", data.author].filter(Boolean).join(", "),
      image_url: imageUrl,
      image_urls: imageUrls,
      platform: "reddit",
    };
  }

  async function redditListings(username) {
    const results = [];
    const seen = new Set();
    let after;
    while (true) {
      const url = new URL(`/user/${encodeURIComponent(username)}/saved.json`, location.origin);
      url.searchParams.set("raw_json", "1");
      url.searchParams.set("limit", "100");
      if (after) url.searchParams.set("after", after);
      const response = await fetch(url.toString(), {
        credentials: "same-origin",
        headers: { "Accept": "application/json" },
      });
      const text = await response.text();
      if (!response.ok) {
        if (response.status === 429) throw new Error("Reddit rate limited the export");
        throw new Error("Reddit " + response.status + ": " + response.statusText);
      }
      if (text.trim().startsWith("<")) throw new Error("Reddit returned HTML instead of saved data — log in to Reddit in this browser");
      let listing;
      try { listing = JSON.parse(text); } catch (e) { throw new Error("Reddit returned invalid JSON"); }
      const children = (listing && listing.data && listing.data.children) || [];
      for (const child of children) {
        if (child.kind !== "t3" && child.kind !== "t1") continue;
        const key = `${child.kind}:${child.data && child.data.id}`;
        if (!child.data || !child.data.id || seen.has(key)) continue;
        seen.add(key);
        results.push(redditItemFromChild(child));
      }
      after = listing && listing.data && listing.data.after;
      if (!after || children.length === 0) break;
    }
    return results;
  }

  // ---------------------------------------------------------------- Instagram
  // DOM-based saved-post scraper. The old /api/v1/feed/saved/posts/ private
  // endpoint rotated/retired (2026): it returns the login wall or nothing when
  // hit from a content script. We're inside the page, so read the rendered
  // DOM instead — every saved post is an <a href="/p/CODE"> or /reel/CODE>
  // with an <img> thumbnail — then scroll the page to reveal the rest.

  async function igExportSavedDom(task) {
    const results = [];
    const seen = new Set();
    const pageText = document.body ? document.body.innerText || "" : "";
    if (/log into instagram|loginrequired|log-in|login.*instagram/i.test(pageText) && pageText.indexOf("saved") < 0) {
      throw new Error("Instagram login required — log in to Instagram in this browser");
    }

    function anchorCode(href) {
      const m = (href || "").match(/^\/(p|reel|tv)\/([\w-]+)/);
      return m ? { kind: m[1], code: m[2] } : null;
    }

    function igItemFromAnchor(a, username) {
      const info = anchorCode(a.getAttribute("href"));
      if (!info) return null;
      // Thumbnail: the anchor's <img> (or a video poster).
      const img = a.querySelector("img");
      const thumb = img ? (img.getAttribute("src") || img.getAttribute("data-src") || "") : "";
      const alt = (img && img.getAttribute("alt")) || "";
      return {
        url: `https://www.instagram.com/${info.kind}/${info.code}/`,
        title: alt.substring(0, 120) || `Instagram ${info.kind === "reel" ? "reel" : "post"} by ${username}`,
        description: alt,
        tags: ["instagram", username].filter(Boolean).join(", "),
        image_url: thumb,
        image_urls: thumb ? [thumb] : [],
        platform: "instagram",
        media_type: info.kind === "reel" ? "video" : "image",
        shortcode: info.code,
      };
    }

    function extractFromPage() {
      const anchors = Array.from(document.querySelectorAll('a[href^="/p/"], a[href^="/reel/"], a[href^="/tv/"]'));
      let found = 0;
      const username = task.username || "";
      for (const a of anchors) {
        const item = igItemFromAnchor(a, username);
        if (!item || seen.has(item.shortcode)) continue;
        seen.add(item.shortcode);
        results.push(item);
        found++;
      }
      return found;
    }

    let lastFound = 0;
    let stagnant = 0;
    let safety = 0;
    while (safety++ < 150) {
      const before = seen.size;
      const added = extractFromPage();
      if (added > 0) lastFound = added;

      // Stop when scrolling two passes adds nothing new (we've hit the end
      // of the saved collection's loaded history).
      if (seen.size === before) {
        stagnant++;
        if (stagnant >= 3) break;
      } else {
        stagnant = 0;
      }

      // Scroll to the bottom of the viewport to trigger IG's infinite load.
      const h = document.documentElement.scrollHeight || document.body.scrollHeight || 0;
      const current = window.scrollY !== undefined ? window.scrollY : (document.documentElement.scrollTop || 0);
      const next = current + (window.innerHeight || 800) * 3;
      if (next < h) {
        (document.documentElement || document.body).scrollTop = next;
      } else if (document.documentElement && document.documentElement.scrollTop < h) {
        document.documentElement.scrollTop = h;
      }
      await sleep(800);
    }

    // If the page is a plain saved page already showing rows, the loop above
    // collects them all. If nothing was found at all, surface it — either the
    // account has zero saved posts or the page didn't render.
    if (results.length === 0) {
      throw new Error("No saved posts found on the Instagram page — check the URL in settings");
    }
    return results;
  }

  // ---------------------------------------------------------------- Pinterest
  async function pinterestFetch(resourceName, options) {
    const data = JSON.stringify({ options, context: {} });
    const sourceUrl = options.source_url || "/";
    const url = `/resource/${resourceName}Resource/get/?source_url=${encodeURIComponent(sourceUrl)}&data=${encodeURIComponent(data)}`;
    const resp = await fetch(url, {
      credentials: "same-origin",
      headers: {
        "Accept": "application/json",
        "X-Pinterest-PWS-Handler": "www/[username].js",
      },
    });
    if (!resp.ok) throw new Error(`Pinterest API ${resp.status}: ${resp.statusText}`);
    const json = await resp.json();
    const rr = json.resource_response;
    if (!rr) return { data: [], bookmark: null };
    const pinData = rr.data || rr;
    const bm = rr.bookmark;
    if (bm === "-end-" || bm === "end" || bm === null || bm === undefined) {
      return { data: Array.isArray(pinData) ? pinData : [], bookmark: null };
    }
    return { data: Array.isArray(pinData) ? pinData : [], bookmark: bm };
  }

  function pinPrimaryImage(images) {
    if (!images) return "";
    const orig = images.orig && images.orig.url;
    const sizes = Object.entries(images)
      .filter(([k]) => k !== "orig")
      .sort((a, b) => ((b[1] && b[1].width) || 0) - ((a[1] && a[1].width) || 0));
    return orig || (sizes[0] && sizes[0][1] && sizes[0][1].url) || "";
  }

  function pinStoryImages(pin, imageUrls) {
    // Idea pins carry extra media under story_pin_data.sections[].
    if (pin.story_pin_data && Array.isArray(pin.story_pin_data.sections)) {
      for (const sec of pin.story_pin_data.sections) {
        const si = sec.story_pin_image && sec.story_pin_image.images;
        if (!si) continue;
        const u = (si.orig && si.orig.url) || (si["736x"] && si["736x"].url) || (si["564x"] && si["564x"].url) || "";
        if (u && !imageUrls.includes(u)) imageUrls.push(u);
      }
    }
  }

  function pinExtract(pin) {
    const primary = pinPrimaryImage(pin.images);
    const imageUrls = primary ? [primary] : [];
    pinStoryImages(pin, imageUrls);
    const pinUrl = `https://www.pinterest.com/pin/${pin.id}/`;
    const title = (pin.grid_title || pin.title || "").trim();
    const desc = (pin.description || "").trim();
    return {
      url: pin.link || pinUrl,
      title: title || `Pin by ${(pin.pinner && pin.pinner.username) || ""}`,
      description: desc || title,
      tags: ["pinterest", pin.pinner && pin.pinner.username, pin.board && pin.board.name].filter(Boolean).join(", "),
      image_url: imageUrls[0] || "",
      image_urls: imageUrls,
      platform: "pinterest",
      pin_url: pinUrl,
    };
  }

  async function pinterestResolveBoardId(username, boardSlug) {
    const resp = await fetch(`/${username}/${boardSlug}/`, { credentials: "same-origin" });
    const html = await resp.text();
    for (const p of [
      /"board_id"\s*:\s*"(\d+)"/,
      /"id"\s*:\s*"(\d+)"\s*,.*?"type"\s*:\s*"board"/,
      /"board":\{[^}]*"id":"(\d+)"/,
    ]) {
      const m = html.match(p);
      if (m) return m[1];
    }
    const sm = html.match(/window\.__INITIAL_STATE__\s*=\s*({.*?});/s);
    if (sm) {
      try {
        const state = JSON.parse(sm[1]);
        return (state.board && state.board.id) || (state.resourceResponses && state.resourceResponses[0] && state.resourceResponses[0].response && state.resourceResponses[0].response.data && state.resourceResponses[0].response.data.id) || null;
      } catch (e) {}
    }
    return null;
  }

  async function pinterestExport(task) {
    const results = [];
    const seen = new Set();
    const filterPins = (data) => (Array.isArray(data) ? data.filter((p) => p && p.id && (!p.type || p.type === "pin")) : []);
    const pushPins = (pins) => {
      for (const pin of pins) {
        const p = pinExtract(pin);
        if (p.pin_url && !seen.has(p.pin_url)) { seen.add(p.pin_url); results.push(p); }
      }
    };

    async function paginate(resourceName, buildOptions) {
      let bookmark = null;
      while (true) {
        const { data, bookmark: nextBm } = await pinterestFetch(resourceName, buildOptions(bookmark));
        const pins = filterPins(data);
        pushPins(pins);
        if (!nextBm || pins.length === 0) break;
        bookmark = nextBm;
      }
    }

    switch (task.type) {
      case "pin": {
        const { data } = await pinterestFetch("Pin", { field_set_key: "detailed", id: task.id, source_url: `/pin/${task.id}/` });
        pushPins(filterPins([(data && data.data) || data]));
        return results;
      }
      case "board": {
        const boardId = task.boardId || await pinterestResolveBoardId(task.username, task.boardSlug);
        if (!boardId) throw new Error(`Could not resolve board: ${task.username}/${task.boardSlug}`);
        await paginate("BoardFeed", (bm) => {
          const o = { board_id: boardId, page_size: 25, source_url: `/${task.username}/${task.boardSlug}/` };
          if (bm) o.bookmarks = [bm];
          return o;
        });
        return results;
      }
      case "profile":
      case "user_created":
        await paginate("UserActivityPins", (bm) => {
          const o = { username: task.username, page_size: 25, source_url: `/${task.username}/_created/` };
          if (bm) o.bookmarks = [bm];
          return o;
        });
        return results;
      case "user_pins":
        await paginate("UserPins", (bm) => {
          const o = { username: task.username, page_size: 25, source_url: `/${task.username}/_pins/` };
          if (bm) o.bookmarks = [bm];
          return o;
        });
        return results;
      default:
        throw new Error("Unsupported Pinterest task type: " + task.type);
    }
  }

  // ---------------------------------------------------------------- entry
  async function runScrape(task) {
    const hostPlatform = platformOfHost();
    if (hostPlatform && task.platform !== hostPlatform) {
      // tolerate: task platform must match the page we're injected on
      throw new Error(`scraper injected on ${hostPlatform} page but task is ${task.platform}`);
    }
    switch (task.platform) {
      case "twitter": return xExportBookmarks();
      case "reddit": return redditListings(task.username);
      case "instagram": return igExportSavedDom(task);
      case "pinterest": return pinterestExport(task);
      default: throw new Error("unknown platform " + task.platform);
    }
  }

  chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
    if (msg && msg.action === "tv-ping") {
      // Attach check from the background worker: respond only when the
      // scraper is live in this document.
      sendResponse({ ok: true });
      return false;
    }
    if (msg && msg.action === "tv-scrape") {
      runScrape(msg.task || {})
        .then((items) => sendResponse({ ok: true, items: items || [] }))
        .catch((e) => sendResponse({ ok: false, error: (e && e.message) || String(e) }));
      return true; // async
    }
  });
}
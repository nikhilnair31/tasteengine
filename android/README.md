# Taste Engine — Android app

Native Android client for a Taste Engine vault: grid, detail, follow-ups,
trash, cleanup, search, and a share target for saving from other apps. Same
crimson-field / obsidian design language as the web app.

## Requirements

- JDK 17
- Android SDK with platform 37 installed
- A running Taste Engine server (see the repository README)

Set the SDK and JDK before you build:

```bash
export ANDROID_HOME="$HOME/Android/Sdk"
export JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64
```

`android/local.properties` is machine-local and gitignored. Create it when your
SDK is not at the default location:

```
sdk.dir=/path/to/Android/Sdk
```

## Build and install

```bash
cd android
./gradlew :app:assembleDebug
# APK: app/build/outputs/apk/debug/app-debug.apk
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

The debug build signs with the standard debug keystore. For a release build,
add your own signing config to `app/build.gradle.kts`.

## Configure

On first start the app shows a login screen. Enter the server URL with its
scheme, plus your `TV_USER` and `TV_PASS`:

- Emulator against a host server: `http://10.0.2.2:8770`
- Phone on the same network: `http://<host-lan-ip>:8770`
- Public or tailnet server: `https://your.host`

The default value is `http://localhost:8770`. That works only when the app runs
on the same device as the server. The app stores the URL, user and password in
app-private `SharedPreferences`.

## What the app does

- **Login** — HTTP Basic, the same credentials as the web app. The server URL
  is editable and stored on the device.
- **Posts** — staggered vault grid with covers, kind/source/hue/tag filter
  groups, 1–4 columns, sort (shuffle / newest / oldest / a→z / z→a / most
  opened), infinite scroll and a "pulling media" shimmer.
- **Follow-ups** — open and done actions found in saved posts: mark done,
  reopen, open the link, jump to the source post. The badge shows the open
  count.
- **Trash** — restore, delete forever, restore all, empty trash.
- **Cleanup** — duplicate groups: review members, toggle the selection, delete
  the selected items, or auto-clean by channel (source / media / empty / all).
- **Search** — the same endpoint as the web app (FTS + semantic + explainer).
- **Save sheet (FAB)** — quick note; text with a URL becomes a link card.
  Optional image attachment.
- **Share target** — share links, text or images from any app.
- **Detail** — media, saved text, explainer sections, follow-ups, links,
  similar items; open source, share, refresh and delete actions.
- **Settings** — server, credentials, dark/light theme, sign out.

## Layout

```
app/src/main/java/com/niknair/tasteengine/
  App.kt            Application: Coil image loader with the auth header
  MainActivity.kt   Compose navigation (grid / detail / settings), share deep-open
  ShareActivity.kt  ACTION_SEND target (text / image / multi-image)
  Prefs.kt          Session store (server, credentials, theme)
  Theme.kt          Design tokens + typography
  net/Api.kt        OkHttp client, all endpoints, Basic auth
  net/Models.kt     JSON to model parsing (org.json)
  vm/VaultVm.kt     Grid, search and save state machine
  ui/*.kt           Login, Grid, Detail, Follow-ups, Trash, Cleanup, Settings
```

Server APIs used (same as the web app): `/api/meta`, `/api/items`,
`/api/search`, `/api/items/{id}` (plus `/refresh`, `/explain`, `/interact`),
`/api/notes`, `/api/items/upload`, `/media/*`.

## Notes

- `minSdk 31`, `compileSdk 37.1`, `targetSdk 36`. Kotlin + Jetpack Compose
  (Material 3), OkHttp, Coil. No Play Services dependencies.
- The app has no iOS counterpart.

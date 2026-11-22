package com.yunofactory.carminium.audio

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.graphics.Color
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.Settings
import android.util.Base64
import android.view.View
import android.webkit.WebSettings
import android.webkit.WebView
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.ActivityResult
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.documentfile.provider.DocumentFile
import app.tauri.PermissionState
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.io.FileOutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.Executors

@InvokeArg
class SetChannelArgs {
    lateinit var channel: Channel
}

@InvokeArg
class TrackMetadataArgs {
    var trackId: String = ""
    var title: String = ""
    var artist: String = ""
    var album: String = ""
    var artworkPath: String? = null
    var durationMs: Double? = null
}

@InvokeArg
class PlayArgs {
    var url: String = ""
    var headers: Map<String, String>? = null
    var metadata: TrackMetadataArgs = TrackMetadataArgs()
    var startPositionMs: Double? = null
    var autoPlay: Boolean? = null
}

@InvokeArg
class SeekArgs {
    var positionMs: Double = 0.0
}

@InvokeArg
class VolumeArgs {
    var level: Float = 1.0f
}

@InvokeArg
class RateArgs {
    var rate: Float = 1.0f
}

@InvokeArg
class PickFilesArgs {
    var destinationDir: String = ""
}

@InvokeArg
class HttpFetchArgs {
    var url: String = ""
    var method: String? = null
    var headers: Map<String, String>? = null
    var body: String? = null
    var timeoutSecs: Double? = null
    var responseBase64: Boolean? = null
}

@TauriPlugin(
    permissions = [
        Permission(alias = "audioMedia", strings = [Manifest.permission.READ_MEDIA_AUDIO]),
        Permission(alias = "legacyStorage", strings = [Manifest.permission.READ_EXTERNAL_STORAGE]),
        Permission(alias = "notifications", strings = [Manifest.permission.POST_NOTIFICATIONS])
    ]
)
class CarminiumAudioPlugin(private val activity: Activity) : Plugin(activity) {
    private val ioExecutor = Executors.newCachedThreadPool()
    private var eventChannel: Channel? = null
    private var webViewRef: WebView? = null
    private var pendingPickDestDir: String = ""
    private var notificationPrompted: Boolean = false
    private var lastInsetTopDp: Int = 0
    private var lastInsetBottomDp: Int = 0

    private val audioExtensions = setOf(
        "mp3", "flac", "m4a", "aac", "wav", "alac", "aif", "aiff", "ogg", "opus", "caf", "mp4"
    )

    override fun load(webView: WebView) {
        super.load(webView)
        webViewRef = webView

        activity.runOnUiThread {
            configureWebView(webView)
            configureEdgeToEdge(webView)
            configureBackHandler(webView)
        }

        CarminiumPlaybackBridge.listener = object : NativeAudioListener {
            override fun onTick(
                positionMs: Double,
                durationMs: Double,
                status: String,
                rate: Double,
                trackId: String
            ) {
                val obj = JSObject().apply {
                    put("type", "tick")
                    put("positionMs", positionMs)
                    put("durationMs", durationMs)
                    put("status", status)
                    put("rate", rate)
                    put("trackId", trackId)
                }
                eventChannel?.send(obj)
            }

            override fun onState(status: String, trackId: String) {
                val obj = JSObject().apply {
                    put("type", "state")
                    put("status", status)
                    put("trackId", trackId)
                }
                eventChannel?.send(obj)
            }

            override fun onEnded(trackId: String) {
                val obj = JSObject().apply {
                    put("type", "ended")
                    put("trackId", trackId)
                }
                eventChannel?.send(obj)
            }

            override fun onRemote(action: String, positionMs: Double?) {
                val obj = JSObject().apply {
                    put("type", "remote")
                    put("action", action)
                    if (positionMs != null) put("positionMs", positionMs)
                }
                eventChannel?.send(obj)
            }

            override fun onError(message: String, trackId: String) {
                val obj = JSObject().apply {
                    put("type", "error")
                    put("message", message)
                    put("trackId", trackId)
                }
                eventChannel?.send(obj)
            }
        }

        CarminiumPlaybackBridge.logDiag("load(webView) ok sdk=${Build.VERSION.SDK_INT}")
    }

    private fun configureWebView(webView: WebView) {
        try {
            webView.overScrollMode = View.OVER_SCROLL_NEVER
            webView.settings.apply {
                setSupportZoom(false)
                builtInZoomControls = false
                displayZoomControls = false
                mixedContentMode = WebSettings.MIXED_CONTENT_ALWAYS_ALLOW
            }
        } catch (_: Exception) {
        }
    }

    @Suppress("DEPRECATION")
    private fun configureEdgeToEdge(webView: WebView) {
        try {
            val window = activity.window
            WindowCompat.setDecorFitsSystemWindows(window, false)
            window.statusBarColor = Color.TRANSPARENT
            window.navigationBarColor = Color.TRANSPARENT
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                window.isNavigationBarContrastEnforced = false
            }

            val root = window.decorView
            ViewCompat.setOnApplyWindowInsetsListener(root) { view, insets ->
                val bars = insets.getInsets(
                    WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
                )
                val density = activity.resources.displayMetrics.density.coerceAtLeast(1f)
                val topDp = (bars.top / density + 0.5f).toInt()
                val bottomDp = (bars.bottom / density + 0.5f).toInt()
                lastInsetTopDp = topDp
                lastInsetBottomDp = bottomDp
                injectSafeAreaInsets(webView, topDp, bottomDp)
                ViewCompat.onApplyWindowInsets(view, insets)
            }
            ViewCompat.requestApplyInsets(root)
        } catch (e: Exception) {
            CarminiumPlaybackBridge.logDiag("edgeToEdge ERR: ${e.message}")
        }
    }

    private fun injectSafeAreaInsets(webView: WebView, topDp: Int, bottomDp: Int) {
        val js = """
            (function(){
              window.__carminiumSafeInsets = { top: $topDp, bottom: $bottomDp };
              var r = document.documentElement;
              if (r) {
                r.style.setProperty('--mobile-safe-top', '${topDp}px');
                r.style.setProperty('--mobile-safe-bottom', '${bottomDp}px');
              }
            })();
        """.trimIndent()
        webView.post {
            try {
                webView.evaluateJavascript(js, null)
            } catch (_: Exception) {
            }
        }
    }

    private fun configureBackHandler(webView: WebView) {
        val comp = activity as? ComponentActivity ?: return
        comp.onBackPressedDispatcher.addCallback(
            comp,
            object : OnBackPressedCallback(true) {
                override fun handleOnBackPressed() {
                    webView.evaluateJavascript(
                        "(function(){return typeof window.__carminiumHandleAndroidBack==='function'?window.__carminiumHandleAndroidBack():'minimize';})()"
                    ) { result ->
                        val cleaned = result?.trim()?.trim('"') ?: "minimize"
                        if (cleaned == "minimize") {
                            activity.moveTaskToBack(true)
                        }
                    }
                }
            }
        )
    }

    private fun maybeRequestNotificationPermission() {
        if (notificationPrompted) return
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        notificationPrompted = true
        val granted = ContextCompat.checkSelfPermission(
            activity,
            Manifest.permission.POST_NOTIFICATIONS
        ) == PackageManager.PERMISSION_GRANTED
        if (!granted) {
            try {
                activity.requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 9012)
            } catch (_: Exception) {
            }
        }
    }

    // MARK: - Tauri Commands

    @Command
    fun setEventChannel(invoke: Invoke) {
        val args = invoke.parseArgs(SetChannelArgs::class.java)
        eventChannel = args.channel
        webViewRef?.let { injectSafeAreaInsets(it, lastInsetTopDp, lastInsetBottomDp) }
        invoke.resolve()
    }

    @Command
    fun play(invoke: Invoke) {
        val args = invoke.parseArgs(PlayArgs::class.java)
        maybeRequestNotificationPermission()
        val payload = PlayPayloadData(
            url = args.url,
            headers = args.headers ?: emptyMap(),
            metadata = TrackMetadataData(
                trackId = args.metadata.trackId,
                title = args.metadata.title,
                artist = args.metadata.artist,
                album = args.metadata.album,
                artworkPath = args.metadata.artworkPath,
                durationMs = args.metadata.durationMs
            ),
            startPositionMs = args.startPositionMs,
            autoPlay = args.autoPlay ?: true
        )
        CarminiumPlaybackBridge.play(activity.applicationContext, payload)
        invoke.resolve()
    }

    @Command
    fun pause(invoke: Invoke) {
        CarminiumPlaybackBridge.pause()
        invoke.resolve()
    }

    @Command
    fun resume(invoke: Invoke) {
        maybeRequestNotificationPermission()
        CarminiumPlaybackBridge.resume(activity.applicationContext)
        invoke.resolve()
    }

    @Command
    fun stop(invoke: Invoke) {
        CarminiumPlaybackBridge.stop()
        invoke.resolve()
    }

    @Command
    fun seek(invoke: Invoke) {
        val args = invoke.parseArgs(SeekArgs::class.java)
        CarminiumPlaybackBridge.seek(args.positionMs)
        invoke.resolve()
    }

    @Command
    fun setVolume(invoke: Invoke) {
        val args = invoke.parseArgs(VolumeArgs::class.java)
        CarminiumPlaybackBridge.setVolume(args.level)
        invoke.resolve()
    }

    @Command
    fun setRate(invoke: Invoke) {
        val args = invoke.parseArgs(RateArgs::class.java)
        CarminiumPlaybackBridge.setRate(args.rate)
        invoke.resolve()
    }

    @Command
    fun updateMetadata(invoke: Invoke) {
        val args = invoke.parseArgs(TrackMetadataArgs::class.java)
        CarminiumPlaybackBridge.updateMetadata(
            TrackMetadataData(
                trackId = args.trackId,
                title = args.title,
                artist = args.artist,
                album = args.album,
                artworkPath = args.artworkPath,
                durationMs = args.durationMs
            )
        )
        invoke.resolve()
    }

    @Command
    fun openAirPlay(invoke: Invoke) {
        activity.runOnUiThread {
            var launched = false
            // Android 11+ System Media Output Switcher dialog (Bluetooth / Speaker / Cast)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                try {
                    val mediaOutputIntent = Intent("com.android.systemui.action.LAUNCH_MEDIA_OUTPUT_DIALOG").apply {
                        setPackage("com.android.systemui")
                        putExtra("package_name", activity.packageName)
                        addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    }
                    activity.startActivity(mediaOutputIntent)
                    launched = true
                } catch (_: Exception) {
                }
            }
            // Fallback 1: Android Media Output Panel / Volume Panel
            if (!launched && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                try {
                    activity.startActivity(Intent(Settings.Panel.ACTION_VOLUME))
                    launched = true
                } catch (_: Exception) {
                }
            }
            // Fallback 2: Show system media volume & output UI
            if (!launched) {
                try {
                    val am = activity.getSystemService(Context.AUDIO_SERVICE) as? AudioManager
                    am?.adjustStreamVolume(AudioManager.STREAM_MUSIC, AudioManager.ADJUST_SAME, AudioManager.FLAG_SHOW_UI)
                } catch (_: Exception) {
                }
            }
            invoke.resolve()
        }
    }

    private fun audioPermissionAlias(): String =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) "audioMedia" else "legacyStorage"

    private fun hasAudioReadPermission(): Boolean {
        val perm = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            Manifest.permission.READ_MEDIA_AUDIO
        } else {
            Manifest.permission.READ_EXTERNAL_STORAGE
        }
        return ContextCompat.checkSelfPermission(activity, perm) == PackageManager.PERMISSION_GRANTED
    }

    @Command
    fun scanDeviceMusic(invoke: Invoke) {
        if (hasAudioReadPermission()) {
            resolveDeviceMusicDirs(invoke)
        } else {
            requestPermissionForAlias(audioPermissionAlias(), invoke, "onAudioPermissionResult")
        }
    }

    @PermissionCallback
    fun onAudioPermissionResult(invoke: Invoke) {
        if (getPermissionState(audioPermissionAlias()) == PermissionState.GRANTED || hasAudioReadPermission()) {
            resolveDeviceMusicDirs(invoke)
        } else {
            val ret = JSObject().apply {
                put("musicDirs", JSArray())
                put("permissionGranted", false)
            }
            invoke.resolve(ret)
        }
    }

    private fun resolveDeviceMusicDirs(invoke: Invoke) {
        val dirs = JSArray()
        val candidates = listOf(
            Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_MUSIC),
            Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
        )
        for (dir in candidates) {
            if (dir != null && dir.exists() && dir.isDirectory && dir.canRead()) {
                // Only add Music unconditionally; add Downloads if it contains audio files
                if (dir.name.equals("Music", ignoreCase = true) || containsAudioFile(dir)) {
                    dirs.put(dir.absolutePath)
                }
            }
        }
        val ret = JSObject().apply {
            put("musicDirs", dirs)
            put("permissionGranted", true)
        }
        invoke.resolve(ret)
    }

    private fun containsAudioFile(dir: File): Boolean {
        val list = dir.listFiles() ?: return false
        for (f in list) {
            if (f.isFile && audioExtensions.contains(f.extension.lowercase(Locale.US))) {
                return true
            }
        }
        return false
    }

    @Command
    fun pickAudioFiles(invoke: Invoke) {
        val args = invoke.parseArgs(PickFilesArgs::class.java)
        pendingPickDestDir = args.destinationDir
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
            )
        }
        startActivityForResult(invoke, intent, "onPickFolderResult")
    }

    @ActivityCallback
    fun onPickFolderResult(invoke: Invoke, result: ActivityResult) {
        val treeUri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || treeUri == null) {
            val canceled = JSObject().apply {
                put("copiedPaths", JSArray())
                put("importedDir", org.json.JSONObject.NULL)
                put("canceled", true)
            }
            invoke.resolve(canceled)
            return
        }

        try {
            activity.contentResolver.takePersistableUriPermission(
                treeUri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION
            )
        } catch (_: Exception) {
        }

        // Fast zero-copy path: if the picked folder resolves to external storage and is readable
        val directPath = resolvePrimaryStoragePath(treeUri)
        if (directPath != null && hasAudioReadPermission()) {
            val dir = File(directPath)
            if (dir.exists() && dir.isDirectory && dir.canRead()) {
                CarminiumPlaybackBridge.logDiag("pickFolder:zeroCopy path=$directPath")
                val ret = JSObject().apply {
                    put("copiedPaths", JSArray())
                    put("importedDir", directPath)
                    put("canceled", false)
                }
                invoke.resolve(ret)
                return
            }
        }

        // Fallback: copy audio files from SAF DocumentTree into app private libraries dir
        val destBase = pendingPickDestDir
        ioExecutor.execute {
            val copied = JSArray()
            var targetDirPath: String? = null
            try {
                val rootDoc = DocumentFile.fromTreeUri(activity, treeUri)
                if (rootDoc != null && rootDoc.exists()) {
                    val baseName = rootDoc.name?.takeIf { it.isNotBlank() }
                        ?: ("导入音乐-" + SimpleDateFormat("yyyyMMdd-HHmmss", Locale.US).format(Date()))
                    val destRoot = File(destBase).apply { mkdirs() }
                    var targetDir = File(destRoot, baseName)
                    var idx = 1
                    while (targetDir.exists()) {
                        targetDir = File(destRoot, "$baseName ($idx)")
                        idx++
                    }
                    targetDir.mkdirs()

                    copyDocumentTreeAudio(rootDoc, targetDir, copied)
                    if (copied.length() > 0) {
                        targetDirPath = targetDir.absolutePath
                    } else {
                        targetDir.deleteRecursively()
                    }
                }
            } catch (e: Exception) {
                CarminiumPlaybackBridge.logDiag("pickFolder SAF copy ERR: ${e.message}")
            }

            activity.runOnUiThread {
                val ret = JSObject().apply {
                    put("copiedPaths", copied)
                    put("importedDir", targetDirPath ?: org.json.JSONObject.NULL)
                    put("canceled", false)
                }
                invoke.resolve(ret)
            }
        }
    }

    private fun resolvePrimaryStoragePath(treeUri: Uri): String? {
        return try {
            val docId = DocumentsContract.getTreeDocumentId(treeUri) ?: return null
            val parts = docId.split(":", limit = 2)
            if (parts.size == 2 && parts[0].equals("primary", ignoreCase = true)) {
                val rel = parts[1].trimStart('/')
                val root = Environment.getExternalStorageDirectory().absolutePath
                if (rel.isEmpty()) root else "$root/$rel"
            } else {
                null
            }
        } catch (_: Exception) {
            null
        }
    }

    private fun copyDocumentTreeAudio(dir: DocumentFile, destDir: File, copied: JSArray) {
        for (child in dir.listFiles()) {
            if (child.isDirectory) {
                copyDocumentTreeAudio(child, destDir, copied)
            } else if (child.isFile) {
                val name = child.name ?: continue
                val ext = name.substringAfterLast('.', "").lowercase(Locale.US)
                if (!audioExtensions.contains(ext)) continue
                try {
                    var target = File(destDir, name)
                    var i = 1
                    while (target.exists()) {
                        val stem = name.substringBeforeLast('.')
                        target = File(destDir, "$stem ($i).$ext")
                        i++
                    }
                    activity.contentResolver.openInputStream(child.uri)?.use { input ->
                        FileOutputStream(target).use { output ->
                            input.copyTo(output, bufferSize = 64 * 1024)
                        }
                    }
                    copied.put(target.absolutePath)
                } catch (_: Exception) {
                }
            }
        }
    }

    @Command
    fun httpFetch(invoke: Invoke) {
        val args = invoke.parseArgs(HttpFetchArgs::class.java)
        ioExecutor.execute {
            val result = JSObject()
            var conn: HttpURLConnection? = null
            try {
                val url = URL(args.url)
                conn = (url.openConnection() as HttpURLConnection).apply {
                    requestMethod = (args.method ?: "GET").uppercase(Locale.US)
                    val timeoutMs = ((args.timeoutSecs ?: 12.0) * 1000.0).toInt().coerceIn(2000, 60000)
                    connectTimeout = timeoutMs
                    readTimeout = timeoutMs
                    instanceFollowRedirects = true
                    args.headers?.forEach { (k, v) -> setRequestProperty(k, v) }
                    if (args.body != null && (requestMethod == "POST" || requestMethod == "PUT" || requestMethod == "PATCH")) {
                        doOutput = true
                        outputStream.use { it.write(args.body!!.toByteArray(Charsets.UTF_8)) }
                    }
                }
                val code = conn.responseCode
                val stream = if (code in 200..399) conn.inputStream else conn.errorStream
                val bytes = stream?.use { it.readBytes() } ?: ByteArray(0)
                val bodyStr = if (args.responseBase64 == true) {
                    Base64.encodeToString(bytes, Base64.NO_WRAP)
                } else {
                    String(bytes, Charsets.UTF_8)
                }
                result.put("status", code)
                result.put("body", bodyStr)
                result.put("error", org.json.JSONObject.NULL)
            } catch (e: Exception) {
                result.put("status", 0)
                result.put("body", "")
                result.put("error", e.message ?: "httpFetch error")
            } finally {
                conn?.disconnect()
            }
            invoke.resolve(result)
        }
    }

    @Command
    fun getDiagnostics(invoke: Invoke) {
        activity.runOnUiThread {
            val lines = mutableListOf<String>()
            lines.add("=== Device ===")
            lines.add("OS: Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})")
            lines.add("Model: ${Build.MANUFACTURER} ${Build.MODEL} (${Build.SUPPORTED_ABIS.joinToString(",")})")
            lines.add("SafeInsets(dp): top=$lastInsetTopDp bottom=$lastInsetBottomDp")

            lines.add("")
            lines.add("=== Package & Signing ===")
            lines.add("Package: ${activity.packageName}")
            val debuggable = (activity.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0
            lines.add("Debuggable: $debuggable  TargetSDK: ${activity.applicationInfo.targetSdkVersion}")
            lines.add("SignSHA256: ${readSigningSha256()}")

            lines.add("")
            lines.add("=== Permissions ===")
            val notifOk = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
            } else true
            lines.add("AudioRead: ${hasAudioReadPermission()}  Notifications: $notifOk")

            lines.add("")
            lines.add("=== AudioManager & ExoPlayer ===")
            val am = activity.getSystemService(Context.AUDIO_SERVICE) as? AudioManager
            val outs = am?.getDevices(AudioManager.GET_DEVICES_OUTPUTS)
                ?.filter {
                    it.type == AudioDeviceInfo.TYPE_BLUETOOTH_A2DP ||
                        it.type == AudioDeviceInfo.TYPE_BLE_HEADSET ||
                        it.type == AudioDeviceInfo.TYPE_WIRED_HEADSET ||
                        it.type == AudioDeviceInfo.TYPE_WIRED_HEADPHONES ||
                        it.type == AudioDeviceInfo.TYPE_USB_HEADSET ||
                        it.type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER
                }
                ?.joinToString(", ") { "${it.type}(${it.productName})" } ?: "<none>"
            lines.add("Outputs: $outs")
            lines.add("Player: ${CarminiumPlaybackBridge.getPlayerSnapshot()}")

            lines.add("")
            val events = CarminiumPlaybackBridge.snapshotDiagEvents()
            lines.add("=== Recent Native Events (${events.size}) ===")
            if (events.isEmpty()) lines.add("(none yet)") else lines.addAll(events)

            val ret = JSObject().apply {
                put("text", lines.joinToString("\n"))
            }
            invoke.resolve(ret)
        }
    }

    @Suppress("DEPRECATION")
    private fun readSigningSha256(): String {
        return try {
            val pm = activity.packageManager
            val sigBytes = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                val info = pm.getPackageInfo(activity.packageName, PackageManager.GET_SIGNING_CERTIFICATES)
                info.signingInfo?.apkContentsSigners?.firstOrNull()?.toByteArray()
            } else {
                val info = pm.getPackageInfo(activity.packageName, PackageManager.GET_SIGNATURES)
                info.signatures?.firstOrNull()?.toByteArray()
            } ?: return "<unknown>"
            val digest = MessageDigest.getInstance("SHA-256").digest(sigBytes)
            digest.joinToString(":") { "%02X".format(it) }
        } catch (e: Exception) {
            "<err:${e.message}>"
        }
    }
}

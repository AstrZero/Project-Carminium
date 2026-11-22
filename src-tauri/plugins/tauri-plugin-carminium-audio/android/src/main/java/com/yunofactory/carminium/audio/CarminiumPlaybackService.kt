package com.yunofactory.carminium.audio

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.annotation.OptIn
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.ForwardingPlayer
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackException
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import java.io.ByteArrayOutputStream
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.Executors

data class TrackMetadataData(
    val trackId: String,
    val title: String,
    val artist: String,
    val album: String,
    val artworkPath: String?,
    val durationMs: Double?
)

data class PlayPayloadData(
    val url: String,
    val headers: Map<String, String>,
    val metadata: TrackMetadataData,
    val startPositionMs: Double?,
    val autoPlay: Boolean
)

interface NativeAudioListener {
    fun onTick(positionMs: Double, durationMs: Double, status: String, rate: Double, trackId: String)
    fun onState(status: String, trackId: String)
    fun onEnded(trackId: String)
    fun onRemote(action: String, positionMs: Double?)
    fun onError(message: String, trackId: String)
}

/**
 * Process-local bridge connecting CarminiumAudioPlugin (Activity/Tauri) and
 * CarminiumPlaybackService (Media3 MediaSessionService foreground service).
 */
@OptIn(UnstableApi::class)
object CarminiumPlaybackBridge {
    private const val TAG = "CarminiumAudio"
    private val mainHandler = Handler(Looper.getMainLooper())
    private val artExecutor = Executors.newSingleThreadExecutor()
    private val timeFmt = SimpleDateFormat("HH:mm:ss.SSS", Locale.US)

    @Volatile
    var listener: NativeAudioListener? = null

    private var serviceRef: CarminiumPlaybackService? = null
    private var exoPlayer: ExoPlayer? = null
    private var mediaSession: MediaSession? = null
    private var pendingActions = mutableListOf<() -> Unit>()

    var currentTrackId: String = ""
        private set
    var currentMetadata: TrackMetadataData? = null
        private set
    var currentDurationMs: Double = 0.0
        private set
    var playbackRate: Float = 1.0f
        private set
    var volumeLevel: Float = 1.0f
        private set
    var isPlayingState: Boolean = false
        private set

    private var artworkGeneration: Int = 0
    private var lastArtworkPath: String? = null
    private var cachedArtworkBytes: ByteArray? = null

    private val diagEvents = ArrayDeque<String>(50)

    private val tickRunnable = object : Runnable {
        override fun run() {
            if (isPlayingState && exoPlayer != null) {
                emitTick()
                mainHandler.postDelayed(this, 250L)
            }
        }
    }

    fun logDiag(msg: String) {
        val line = "[${timeFmt.format(Date())}] $msg"
        runOnMain {
            if (diagEvents.size >= 45) {
                diagEvents.removeFirst()
            }
            diagEvents.addLast(line)
        }
    }

    fun snapshotDiagEvents(): List<String> = synchronized(diagEvents) { diagEvents.toList() }

    fun getPlayerSnapshot(): String {
        val p = exoPlayer
        return if (p != null) {
            "exists state=${p.playbackState} isPlaying=${p.isPlaying} playWhenReady=${p.playWhenReady} " +
                "pos=${p.currentPosition} dur=${p.duration} rate=${p.playbackParameters.speed} " +
                "sessionActive=${mediaSession != null}"
        } else {
            "<nil> serviceStarted=${serviceRef != null}"
        }
    }

    fun runOnMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            block()
        } else {
            mainHandler.post(block)
        }
    }

    private fun ensureServiceAndRun(context: Context, action: () -> Unit) {
        runOnMain {
            if (exoPlayer != null && serviceRef != null) {
                action()
            } else {
                pendingActions.add(action)
                val appCtx = context.applicationContext
                val intent = Intent(appCtx, CarminiumPlaybackService::class.java)
                try {
                    appCtx.startService(intent)
                } catch (e: Exception) {
                    Log.w(TAG, "startService failed, retrying foreground: ${e.message}")
                    try {
                        appCtx.startForegroundService(intent)
                    } catch (e2: Exception) {
                        logDiag("service start ERR: ${e2.message}")
                    }
                }
            }
        }
    }

    internal fun attachService(
        service: CarminiumPlaybackService,
        player: ExoPlayer,
        session: MediaSession
    ) {
        runOnMain {
            serviceRef = service
            exoPlayer = player
            mediaSession = session
            player.volume = volumeLevel
            player.playbackParameters = PlaybackParameters(playbackRate)
            logDiag("service:attached MediaSession+ExoPlayer")

            val queued = pendingActions.toList()
            pendingActions.clear()
            queued.forEach { it.invoke() }
        }
    }

    internal fun detachService(service: CarminiumPlaybackService) {
        runOnMain {
            if (serviceRef === service) {
                mainHandler.removeCallbacks(tickRunnable)
                serviceRef = null
                exoPlayer = null
                mediaSession = null
                isPlayingState = false
                logDiag("service:detached")
            }
        }
    }

    fun play(context: Context, payload: PlayPayloadData) {
        ensureServiceAndRun(context) {
            val player = exoPlayer ?: return@ensureServiceAndRun
            val svc = serviceRef ?: return@ensureServiceAndRun

            val uri = resolveUri(payload.url)
            if (uri == null) {
                logDiag("play ERR invalid url: ${payload.url}")
                listener?.onError("Invalid audio URL: ${payload.url}", payload.metadata.trackId)
                return@ensureServiceAndRun
            }

            currentTrackId = payload.metadata.trackId
            currentMetadata = payload.metadata
            currentDurationMs = payload.metadata.durationMs ?: 0.0
            logDiag("play id=${payload.metadata.trackId} autoPlay=${payload.autoPlay}")

            val artPath = payload.metadata.artworkPath?.takeIf { it.isNotEmpty() }
            val immediateArt = if (artPath != null && artPath == lastArtworkPath) cachedArtworkBytes else null
            val mediaMeta = buildMediaMetadata(payload.metadata, immediateArt)

            val mediaItem = MediaItem.Builder()
                .setMediaId(payload.metadata.trackId)
                .setUri(uri)
                .setMediaMetadata(mediaMeta)
                .build()

            val httpFactory = DefaultHttpDataSource.Factory()
                .setAllowCrossProtocolRedirects(true)
                .setConnectTimeoutMs(15_000)
                .setReadTimeoutMs(15_000)
                .setUserAgent("Carminium/1.2 (Android)")
            if (payload.headers.isNotEmpty()) {
                httpFactory.setDefaultRequestProperties(payload.headers)
            }
            val dataSourceFactory = DefaultDataSource.Factory(svc, httpFactory)
            val mediaSource = DefaultMediaSourceFactory(dataSourceFactory).createMediaSource(mediaItem)

            player.setMediaSource(mediaSource)
            player.playbackParameters = PlaybackParameters(playbackRate)
            player.volume = volumeLevel
            player.prepare()

            val startMs = payload.startPositionMs
            if (startMs != null && startMs > 0.0) {
                player.seekTo(startMs.toLong())
            }

            if (payload.autoPlay) {
                player.playWhenReady = true
                player.play()
                isPlayingState = true
                scheduleTicks()
                listener?.onState("playing", currentTrackId)
                emitTick()
            } else {
                player.playWhenReady = false
                player.pause()
                isPlayingState = false
                stopTicks()
                listener?.onState("paused", currentTrackId)
                emitTick()
            }

            loadArtworkAsync(payload.metadata, artPath)
        }
    }

    fun pause() {
        runOnMain {
            val player = exoPlayer ?: return@runOnMain
            logDiag("cmd:pause")
            player.pause()
            isPlayingState = false
            stopTicks()
            listener?.onState("paused", currentTrackId)
            emitTick()
        }
    }

    fun resume(context: Context) {
        ensureServiceAndRun(context) {
            val player = exoPlayer ?: return@ensureServiceAndRun
            if (player.currentMediaItem == null) return@ensureServiceAndRun
            logDiag("cmd:resume")
            player.playbackParameters = PlaybackParameters(playbackRate)
            player.play()
            isPlayingState = true
            scheduleTicks()
            listener?.onState("playing", currentTrackId)
            emitTick()
        }
    }

    fun stop() {
        runOnMain {
            logDiag("cmd:stop")
            artworkGeneration += 1
            lastArtworkPath = null
            cachedArtworkBytes = null
            exoPlayer?.stop()
            exoPlayer?.clearMediaItems()
            isPlayingState = false
            currentTrackId = ""
            currentMetadata = null
            currentDurationMs = 0.0
            stopTicks()
            listener?.onState("stopped", "")
        }
    }

    fun seek(positionMs: Double) {
        runOnMain {
            val player = exoPlayer ?: return@runOnMain
            val clamped = positionMs.coerceAtLeast(0.0).toLong()
            player.seekTo(clamped)
            emitTick(overridePositionMs = clamped.toDouble())
        }
    }

    fun setVolume(level: Float) {
        runOnMain {
            volumeLevel = level.coerceIn(0.0f, 1.0f)
            exoPlayer?.volume = volumeLevel
        }
    }

    fun setRate(rate: Float) {
        runOnMain {
            playbackRate = rate.coerceIn(0.5f, 2.0f)
            exoPlayer?.playbackParameters = PlaybackParameters(playbackRate)
        }
    }

    fun updateMetadata(metadata: TrackMetadataData) {
        runOnMain {
            currentMetadata = metadata
            if (metadata.durationMs != null && metadata.durationMs > 0.0) {
                currentDurationMs = metadata.durationMs
            }
            val artPath = metadata.artworkPath?.takeIf { it.isNotEmpty() }
            val immediateArt = if (artPath != null && artPath == lastArtworkPath) cachedArtworkBytes else null
            applyMediaMetadataToCurrentItem(buildMediaMetadata(metadata, immediateArt))
            loadArtworkAsync(metadata, artPath)
        }
    }

    internal fun handlePlayerIsPlayingChanged(playing: Boolean) {
        runOnMain {
            logDiag("exo:isPlaying=$playing")
            if (currentTrackId.isEmpty()) return@runOnMain
            if (playing != isPlayingState) {
                isPlayingState = playing
                if (playing) scheduleTicks() else stopTicks()
                listener?.onState(if (playing) "playing" else "paused", currentTrackId)
                emitTick()
            }
        }
    }

    internal fun handlePlayerStateChanged(state: Int) {
        runOnMain {
            val player = exoPlayer ?: return@runOnMain
            when (state) {
                Player.STATE_READY -> {
                    val d = player.duration
                    if (d != C.TIME_UNSET && d > 0L) {
                        currentDurationMs = d.toDouble()
                    }
                    emitTick()
                }
                Player.STATE_ENDED -> {
                    logDiag("exo:ended id=$currentTrackId")
                    isPlayingState = false
                    stopTicks()
                    if (currentTrackId.isNotEmpty()) {
                        listener?.onEnded(currentTrackId)
                    }
                }
            }
        }
    }

    internal fun handlePlayerError(error: PlaybackException) {
        runOnMain {
            val msg = error.message ?: "ExoPlayer playback error (${error.errorCodeName})"
            logDiag("exo:error $msg")
            isPlayingState = false
            stopTicks()
            listener?.onError(msg, currentTrackId)
            listener?.onState("paused", currentTrackId)
        }
    }

    internal fun emitRemote(action: String, positionMs: Double?) {
        runOnMain {
            logDiag("remote:$action${if (positionMs != null) "(${positionMs.toLong()}ms)" else ""}")
            listener?.onRemote(action, positionMs)
        }
    }

    internal fun emitTick(overridePositionMs: Double? = null) {
        val l = listener ?: return
        val player = exoPlayer
        val posMs = overridePositionMs ?: (player?.currentPosition?.coerceAtLeast(0L)?.toDouble() ?: 0.0)
        val rawDur = player?.duration ?: C.TIME_UNSET
        val durMs = if (rawDur != C.TIME_UNSET && rawDur > 0L) rawDur.toDouble() else currentDurationMs
        val status = when {
            isPlayingState -> "playing"
            currentTrackId.isEmpty() -> "stopped"
            else -> "paused"
        }
        val rate = if (isPlayingState) playbackRate.toDouble() else 0.0
        l.onTick(posMs, durMs, status, rate, currentTrackId)
    }

    private fun scheduleTicks() {
        mainHandler.removeCallbacks(tickRunnable)
        mainHandler.postDelayed(tickRunnable, 250L)
    }

    private fun stopTicks() {
        mainHandler.removeCallbacks(tickRunnable)
    }

    private fun resolveUri(raw: String): Uri? {
        if (raw.isBlank()) return null
        return when {
            raw.startsWith("http://") || raw.startsWith("https://") ||
                raw.startsWith("content://") || raw.startsWith("file://") -> Uri.parse(raw)
            else -> Uri.fromFile(File(raw))
        }
    }

    private fun buildMediaMetadata(meta: TrackMetadataData, artworkBytes: ByteArray?): MediaMetadata {
        val builder = MediaMetadata.Builder()
            .setTitle(meta.title)
            .setArtist(meta.artist)
            .setAlbumTitle(meta.album)
            .setDisplayTitle(meta.title)
            .setSubtitle(meta.artist)
        if (artworkBytes != null && artworkBytes.isNotEmpty()) {
            builder.setArtworkData(artworkBytes, MediaMetadata.PICTURE_TYPE_FRONT_COVER)
        }
        return builder.build()
    }

    private fun applyMediaMetadataToCurrentItem(meta: MediaMetadata) {
        val player = exoPlayer ?: return
        val current = player.currentMediaItem ?: return
        val idx = player.currentMediaItemIndex
        if (idx < 0 || idx >= player.mediaItemCount) return
        val updated = current.buildUpon().setMediaMetadata(meta).build()
        player.replaceMediaItem(idx, updated)
    }

    private fun loadArtworkAsync(meta: TrackMetadataData, artPath: String?) {
        if (artPath == null) {
            artworkGeneration += 1
            lastArtworkPath = null
            cachedArtworkBytes = null
            return
        }
        if (artPath == lastArtworkPath && cachedArtworkBytes != null) {
            return
        }
        artworkGeneration += 1
        val gen = artworkGeneration
        artExecutor.execute {
            val bytes = decodeScaledArtworkJpeg(artPath, 600)
            runOnMain {
                if (artworkGeneration != gen) return@runOnMain
                if (bytes != null) {
                    lastArtworkPath = artPath
                    cachedArtworkBytes = bytes
                    val latest = currentMetadata ?: meta
                    applyMediaMetadataToCurrentItem(buildMediaMetadata(latest, bytes))
                }
            }
        }
    }

    private fun decodeScaledArtworkJpeg(path: String, maxDim: Int): ByteArray? {
        return try {
            val f = File(path)
            if (!f.exists() || !f.canRead()) return null
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeFile(path, bounds)
            val srcW = bounds.outWidth
            val srcH = bounds.outHeight
            if (srcW <= 0 || srcH <= 0) return null

            var sample = 1
            while ((srcW / sample) > maxDim * 2 || (srcH / sample) > maxDim * 2) {
                sample *= 2
            }
            val opts = BitmapFactory.Options().apply { inSampleSize = sample }
            val bmp = BitmapFactory.decodeFile(path, opts) ?: return null
            val finalBmp = if (bmp.width > maxDim || bmp.height > maxDim) {
                val scale = minOf(maxDim.toFloat() / bmp.width, maxDim.toFloat() / bmp.height)
                val tw = (bmp.width * scale).toInt().coerceAtLeast(1)
                val th = (bmp.height * scale).toInt().coerceAtLeast(1)
                val scaled = Bitmap.createScaledBitmap(bmp, tw, th, true)
                if (scaled !== bmp) bmp.recycle()
                scaled
            } else {
                bmp
            }
            val out = ByteArrayOutputStream()
            finalBmp.compress(Bitmap.CompressFormat.JPEG, 88, out)
            finalBmp.recycle()
            out.toByteArray()
        } catch (_: Exception) {
            null
        }
    }
}

@OptIn(UnstableApi::class)
class CarminiumPlaybackService : MediaSessionService() {
    private var mediaSession: MediaSession? = null
    private var exoPlayer: ExoPlayer? = null

    private class CarminiumForwardingPlayer(player: Player) : ForwardingPlayer(player) {
        override fun getAvailableCommands(): Player.Commands {
            return super.getAvailableCommands()
                .buildUpon()
                .addAll(
                    COMMAND_SEEK_TO_NEXT,
                    COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
                    COMMAND_SEEK_TO_PREVIOUS,
                    COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM
                )
                .build()
        }

        override fun isCommandAvailable(command: Int): Boolean {
            return when (command) {
                COMMAND_SEEK_TO_NEXT,
                COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
                COMMAND_SEEK_TO_PREVIOUS,
                COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM -> true
                else -> super.isCommandAvailable(command)
            }
        }

        override fun hasNextMediaItem(): Boolean = true
        override fun hasPreviousMediaItem(): Boolean = true

        override fun seekToNext() {
            CarminiumPlaybackBridge.emitRemote("next", null)
        }

        override fun seekToNextMediaItem() {
            CarminiumPlaybackBridge.emitRemote("next", null)
        }

        override fun seekToPrevious() {
            CarminiumPlaybackBridge.emitRemote("prev", null)
        }

        override fun seekToPreviousMediaItem() {
            CarminiumPlaybackBridge.emitRemote("prev", null)
        }

        override fun seekTo(positionMs: Long) {
            super.seekTo(positionMs)
            CarminiumPlaybackBridge.emitRemote("seek", positionMs.toDouble())
        }
    }

    override fun onCreate() {
        super.onCreate()
        val audioAttributes = AudioAttributes.Builder()
            .setUsage(C.USAGE_MEDIA)
            .setContentType(C.AUDIO_CONTENT_TYPE_MUSIC)
            .build()

        val player = ExoPlayer.Builder(this)
            .setAudioAttributes(audioAttributes, /* handleAudioFocus = */ true)
            .setHandleAudioBecomingNoisy(true)
            .setWakeMode(C.WAKE_MODE_NETWORK)
            .build()

        player.addListener(object : Player.Listener {
            override fun onIsPlayingChanged(isPlaying: Boolean) {
                CarminiumPlaybackBridge.handlePlayerIsPlayingChanged(isPlaying)
            }

            override fun onPlaybackStateChanged(playbackState: Int) {
                CarminiumPlaybackBridge.handlePlayerStateChanged(playbackState)
            }

            override fun onPlayerError(error: PlaybackException) {
                CarminiumPlaybackBridge.handlePlayerError(error)
            }
        })

        val forwarding = CarminiumForwardingPlayer(player)
        val launchIntent = packageManager.getLaunchIntentForPackage(packageName)?.apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val sessionBuilder = MediaSession.Builder(this, forwarding)
        if (launchIntent != null) {
            val pi = PendingIntent.getActivity(
                this,
                0,
                launchIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
            )
            sessionBuilder.setSessionActivity(pi)
        }
        val session = sessionBuilder.build()

        exoPlayer = player
        mediaSession = session
        CarminiumPlaybackBridge.attachService(this, player, session)
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? {
        return mediaSession
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        val player = exoPlayer
        if (player == null || !player.playWhenReady || player.mediaItemCount == 0) {
            stopSelf()
        }
    }

    override fun onDestroy() {
        CarminiumPlaybackBridge.detachService(this)
        mediaSession?.release()
        mediaSession = null
        exoPlayer?.release()
        exoPlayer = null
        super.onDestroy()
    }
}

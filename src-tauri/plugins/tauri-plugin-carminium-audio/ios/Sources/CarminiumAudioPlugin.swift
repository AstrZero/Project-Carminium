import AVFoundation
import Foundation
import MediaPlayer
import SwiftRs
import Tauri
import WebKit

#if os(iOS)
import AVKit
import UIKit
import UniformTypeIdentifiers
#endif

// MARK: - Request / Event Models

private struct SetChannelArgs: Decodable {
  let channel: Channel
}

private struct TrackMetadataArgs: Decodable {
  let trackId: String
  let title: String
  let artist: String
  let album: String
  let artworkPath: String?
  let durationMs: Double?
}

private struct PlayArgs: Decodable {
  let url: String
  let headers: [String: String]?
  let metadata: TrackMetadataArgs
  let startPositionMs: Double?
  let autoPlay: Bool?
}

private struct SeekArgs: Decodable {
  let positionMs: Double
}

private struct VolumeArgs: Decodable {
  let level: Float
}

private struct RateArgs: Decodable {
  let rate: Float
}

private struct PickFilesArgs: Decodable {
  let destinationDir: String
}

private struct PickFilesResult: Encodable {
  let copiedPaths: [String]
  let canceled: Bool
}

private struct TickEvent: Encodable {
  let type = "tick"
  let positionMs: Double
  let durationMs: Double
  let status: String
  let rate: Double
  let trackId: String
}

private struct StateEvent: Encodable {
  let type = "state"
  let status: String
  let trackId: String
}

private struct EndedEvent: Encodable {
  let type = "ended"
  let trackId: String
}

private struct RemoteEvent: Encodable {
  let type = "remote"
  let action: String
  let positionMs: Double?
}

private struct ErrorEvent: Encodable {
  let type = "error"
  let message: String
  let trackId: String
}

// MARK: - Plugin Implementation

class CarminiumAudioPlugin: Plugin {
  private var player: AVPlayer?
  private var timeObserver: Any?
  private var endObserver: NSObjectProtocol?
  private var failObserver: NSObjectProtocol?
  private var statusObservation: NSKeyValueObservation?
  private var eventChannel: Channel?

  private var currentTrackId: String = ""
  private var currentMetadata: TrackMetadataArgs?
  private var currentDurationMs: Double = 0
  private var playbackRate: Float = 1.0
  private var volumeLevel: Float = 1.0
  private var isPlayingState: Bool = false
  private var wasInterruptedWhilePlaying: Bool = false
  private var audioSessionConfigured: Bool = false
  private var remoteCommandsConfigured: Bool = false

  #if os(iOS)
  private var routePickerView: AVRoutePickerView?
  private var pendingPickInvoke: Invoke?
  private var pendingPickDestination: URL?
  #endif

  override init() {
    super.init()
    #if os(iOS)
    registerAudioSessionNotifications()
    #endif
  }

  deinit {
    cleanupPlayerObservers()
    if let obs = timeObserver {
      player?.removeTimeObserver(obs)
      timeObserver = nil
    }
    NotificationCenter.default.removeObserver(self)
  }

  // MARK: - Tauri Commands

  @objc public func setEventChannel(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(SetChannelArgs.self)
    DispatchQueue.main.async {
      self.eventChannel = args.channel
      invoke.resolve()
    }
  }

  @objc public func play(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PlayArgs.self)
    DispatchQueue.main.async {
      self.handlePlay(args)
      invoke.resolve()
    }
  }

  @objc public func pause(_ invoke: Invoke) {
    DispatchQueue.main.async {
      self.player?.pause()
      self.isPlayingState = false
      self.updateNowPlayingPlaybackState(isPlaying: false)
      self.sendStateEvent("paused")
      self.sendTickEvent()
      invoke.resolve()
    }
  }

  @objc public func resume(_ invoke: Invoke) {
    DispatchQueue.main.async {
      guard let player = self.player, player.currentItem != nil else {
        invoke.resolve()
        return
      }
      #if os(iOS)
      self.ensureAudioSessionActive()
      #endif
      player.playImmediately(atRate: self.playbackRate)
      self.isPlayingState = true
      self.updateNowPlayingPlaybackState(isPlaying: true)
      self.sendStateEvent("playing")
      self.sendTickEvent()
      invoke.resolve()
    }
  }

  @objc public func stop(_ invoke: Invoke) {
    DispatchQueue.main.async {
      self.cleanupPlayerObservers()
      self.player?.pause()
      self.player?.replaceCurrentItem(with: nil)
      self.isPlayingState = false
      self.currentTrackId = ""
      self.currentMetadata = nil
      self.currentDurationMs = 0
      MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
      self.sendStateEvent("stopped")
      invoke.resolve()
    }
  }

  @objc public func seek(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(SeekArgs.self)
    DispatchQueue.main.async {
      self.performSeek(positionMs: args.positionMs)
      invoke.resolve()
    }
  }

  @objc public func setVolume(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(VolumeArgs.self)
    DispatchQueue.main.async {
      self.volumeLevel = max(0.0, min(1.0, args.level))
      self.player?.volume = self.volumeLevel
      invoke.resolve()
    }
  }

  @objc public func setRate(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(RateArgs.self)
    DispatchQueue.main.async {
      let clamped = max(0.5, min(2.0, args.rate))
      self.playbackRate = clamped
      if self.isPlayingState {
        self.player?.rate = clamped
        self.updateNowPlayingPlaybackState(isPlaying: true)
      }
      invoke.resolve()
    }
  }

  @objc public func updateMetadata(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(TrackMetadataArgs.self)
    DispatchQueue.main.async {
      self.currentMetadata = args
      if let d = args.durationMs, d > 0 {
        self.currentDurationMs = d
      }
      self.applyNowPlayingInfo(metadata: args, isPlaying: self.isPlayingState)
      invoke.resolve()
    }
  }

  #if os(iOS)
  private func resolveTopViewController(_ invoke: Invoke) -> UIViewController? {
    if let vc = invoke.presentingViewController() {
      return vc
    }
    let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
    let window = scenes.flatMap { $0.windows }.first { $0.isKeyWindow }
      ?? scenes.flatMap { $0.windows }.first
    var top = window?.rootViewController
    while let presented = top?.presentedViewController {
      top = presented
    }
    return top
  }
  #endif

  @objc public func openAirPlay(_ invoke: Invoke) {
    #if os(iOS)
    DispatchQueue.main.async {
      guard let vc = self.resolveTopViewController(invoke) else {
        invoke.reject("No presenting view controller available")
        return
      }
      let picker: AVRoutePickerView
      if let existing = self.routePickerView {
        picker = existing
      } else {
        let created = AVRoutePickerView(frame: CGRect(x: -100, y: -100, width: 44, height: 44))
        created.isHidden = true
        created.prioritizesVideoDevices = false
        self.routePickerView = created
        picker = created
      }
      if picker.superview == nil {
        vc.view.addSubview(picker)
      }
      for subview in picker.subviews {
        if let button = subview as? UIButton {
          button.sendActions(for: .touchUpInside)
          break
        }
      }
      invoke.resolve()
    }
    #else
    invoke.resolve()
    #endif
  }

  @objc public func pickAudioFiles(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PickFilesArgs.self)
    #if os(iOS)
    DispatchQueue.main.async {
      guard let vc = self.resolveTopViewController(invoke) else {
        invoke.resolve(PickFilesResult(copiedPaths: [], canceled: true))
        return
      }
      let dest = URL(fileURLWithPath: args.destinationDir, isDirectory: true)
      try? FileManager.default.createDirectory(at: dest, withIntermediateDirectories: true)

      self.pendingPickInvoke = invoke
      self.pendingPickDestination = dest

      let types: [UTType] = [.audio, .folder]
      let picker = UIDocumentPickerViewController(forOpeningContentTypes: types, asCopy: false)
      picker.allowsMultipleSelection = true
      picker.delegate = self
      vc.present(picker, animated: true)
    }
    #else
    invoke.resolve(PickFilesResult(copiedPaths: [], canceled: true))
    #endif
  }

  // MARK: - Core AVPlayer Logic

  private func handlePlay(_ args: PlayArgs) {
    #if os(iOS)
    ensureAudioSessionActive()
    #endif
    ensureRemoteCommandsConfigured()

    guard let mediaURL = resolveURL(args.url) else {
      sendErrorEvent("Invalid audio URL: \(args.url)", trackId: args.metadata.trackId)
      return
    }

    cleanupPlayerObservers()

    let asset: AVURLAsset
    if let headers = args.headers, !headers.isEmpty {
      asset = AVURLAsset(url: mediaURL, options: ["AVURLAssetHTTPHeaderFieldsKey": headers])
    } else {
      asset = AVURLAsset(url: mediaURL)
    }

    let item = AVPlayerItem(asset: asset)
    item.preferredForwardBufferDuration = 10.0

    if self.player == nil {
      let p = AVPlayer(playerItem: item)
      p.automaticallyWaitsToMinimizeStalling = true
      p.volume = self.volumeLevel
      self.player = p
      installPeriodicTimeObserver(on: p)
    } else {
      self.player?.replaceCurrentItem(with: item)
      self.player?.volume = self.volumeLevel
    }

    self.currentTrackId = args.metadata.trackId
    self.currentMetadata = args.metadata
    self.currentDurationMs = args.metadata.durationMs ?? 0

    statusObservation = item.observe(\.status, options: [.new]) { [weak self] observedItem, _ in
      DispatchQueue.main.async {
        guard let self = self else { return }
        switch observedItem.status {
        case .readyToPlay:
          let itemDur = observedItem.duration.seconds
          if itemDur.isFinite && itemDur > 0 {
            self.currentDurationMs = itemDur * 1000.0
          }
          if let meta = self.currentMetadata {
            self.applyNowPlayingInfo(metadata: meta, isPlaying: self.isPlayingState)
          }
          self.sendTickEvent()
        case .failed:
          let msg = observedItem.error?.localizedDescription ?? "AVPlayerItem failed to load"
          self.isPlayingState = false
          self.sendErrorEvent(msg, trackId: self.currentTrackId)
          self.sendStateEvent("paused")
        default:
          break
        }
      }
    }

    endObserver = NotificationCenter.default.addObserver(
      forName: .AVPlayerItemDidPlayToEndTime,
      object: item,
      queue: .main
    ) { [weak self] _ in
      guard let self = self else { return }
      self.isPlayingState = false
      self.sendEndedEvent(trackId: self.currentTrackId)
    }

    failObserver = NotificationCenter.default.addObserver(
      forName: .AVPlayerItemFailedToPlayToEndTime,
      object: item,
      queue: .main
    ) { [weak self] note in
      guard let self = self else { return }
      let err = note.userInfo?[AVPlayerItemFailedToPlayToEndTimeErrorKey] as? Error
      self.isPlayingState = false
      self.sendErrorEvent(
        err?.localizedDescription ?? "Playback failed before end",
        trackId: self.currentTrackId
      )
      self.sendStateEvent("paused")
    }

    let shouldAutoPlay = args.autoPlay ?? true
    if let startMs = args.startPositionMs, startMs > 0 {
      let target = CMTime(seconds: startMs / 1000.0, preferredTimescale: 1000)
      self.player?.seek(to: target, toleranceBefore: .zero, toleranceAfter: .zero) { [weak self] _ in
        guard let self = self else { return }
        if shouldAutoPlay {
          self.player?.playImmediately(atRate: self.playbackRate)
          self.isPlayingState = true
          self.applyNowPlayingInfo(metadata: args.metadata, isPlaying: true)
          self.sendStateEvent("playing")
          self.sendTickEvent()
        }
      }
    } else if shouldAutoPlay {
      self.player?.playImmediately(atRate: self.playbackRate)
      self.isPlayingState = true
      self.applyNowPlayingInfo(metadata: args.metadata, isPlaying: true)
      self.sendStateEvent("playing")
      self.sendTickEvent()
    } else {
      self.player?.pause()
      self.isPlayingState = false
      self.applyNowPlayingInfo(metadata: args.metadata, isPlaying: false)
      self.sendStateEvent("paused")
      self.sendTickEvent()
    }
  }

  private func performSeek(positionMs: Double) {
    guard let player = self.player, player.currentItem != nil else { return }
    let clampedMs = max(0.0, positionMs)
    let target = CMTime(seconds: clampedMs / 1000.0, preferredTimescale: 1000)
    player.seek(to: target, toleranceBefore: .zero, toleranceAfter: .zero) { [weak self] _ in
      guard let self = self else { return }
      self.updateNowPlayingPlaybackState(isPlaying: self.isPlayingState, overrideElapsedSec: clampedMs / 1000.0)
      self.sendTickEvent(overridePositionMs: clampedMs)
    }
  }

  private func installPeriodicTimeObserver(on player: AVPlayer) {
    let interval = CMTime(seconds: 0.25, preferredTimescale: 600)
    timeObserver = player.addPeriodicTimeObserver(forInterval: interval, queue: .main) { [weak self] _ in
      guard let self = self else { return }
      if self.isPlayingState {
        self.sendTickEvent()
      }
    }
  }

  private func cleanupPlayerObservers() {
    statusObservation?.invalidate()
    statusObservation = nil
    if let obs = endObserver {
      NotificationCenter.default.removeObserver(obs)
      endObserver = nil
    }
    if let obs = failObserver {
      NotificationCenter.default.removeObserver(obs)
      failObserver = nil
    }
  }

  private func resolveURL(_ raw: String) -> URL? {
    if raw.hasPrefix("http://") || raw.hasPrefix("https://") {
      if let direct = URL(string: raw) {
        return direct
      }
      if let encoded = raw.addingPercentEncoding(withAllowedCharacters: .urlFragmentAllowed) {
        return URL(string: encoded)
      }
      return nil
    }
    if raw.hasPrefix("file://") {
      if let direct = URL(string: raw) {
        return direct
      }
      let stripped = String(raw.dropFirst("file://".count))
      return URL(fileURLWithPath: stripped)
    }
    return URL(fileURLWithPath: raw)
  }

  // MARK: - AVAudioSession (iOS Background Playback + Interruptions + Route Change)

  #if os(iOS)
  private func ensureAudioSessionActive() {
    let session = AVAudioSession.sharedInstance()
    do {
      if !audioSessionConfigured {
        try session.setCategory(.playback, mode: .default, options: [])
        audioSessionConfigured = true
      }
      try session.setActive(true)
    } catch {
      NSLog("[CarminiumAudio] AVAudioSession activation error: \(error.localizedDescription)")
    }
  }

  private func registerAudioSessionNotifications() {
    let nc = NotificationCenter.default
    nc.addObserver(
      self,
      selector: #selector(handleAudioInterruption(_:)),
      name: AVAudioSession.interruptionNotification,
      object: AVAudioSession.sharedInstance()
    )
    nc.addObserver(
      self,
      selector: #selector(handleAudioRouteChange(_:)),
      name: AVAudioSession.routeChangeNotification,
      object: AVAudioSession.sharedInstance()
    )
  }

  @objc private func handleAudioInterruption(_ notification: Notification) {
    guard
      let info = notification.userInfo,
      let typeVal = info[AVAudioSessionInterruptionTypeKey] as? UInt,
      let type = AVAudioSession.InterruptionType(rawValue: typeVal)
    else { return }

    DispatchQueue.main.async {
      switch type {
      case .began:
        self.wasInterruptedWhilePlaying = self.isPlayingState
        if self.isPlayingState {
          self.player?.pause()
          self.isPlayingState = false
          self.updateNowPlayingPlaybackState(isPlaying: false)
          self.sendStateEvent("paused")
        }
      case .ended:
        let optVal = info[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0
        let options = AVAudioSession.InterruptionOptions(rawValue: optVal)
        if self.wasInterruptedWhilePlaying && options.contains(.shouldResume) {
          self.ensureAudioSessionActive()
          self.player?.playImmediately(atRate: self.playbackRate)
          self.isPlayingState = true
          self.updateNowPlayingPlaybackState(isPlaying: true)
          self.sendStateEvent("playing")
        }
        self.wasInterruptedWhilePlaying = false
      @unknown default:
        break
      }
    }
  }

  @objc private func handleAudioRouteChange(_ notification: Notification) {
    guard
      let info = notification.userInfo,
      let reasonVal = info[AVAudioSessionRouteChangeReasonKey] as? UInt,
      let reason = AVAudioSession.RouteChangeReason(rawValue: reasonVal)
    else { return }

    if reason == .oldDeviceUnavailable {
      DispatchQueue.main.async {
        if self.isPlayingState {
          self.player?.pause()
          self.isPlayingState = false
          self.updateNowPlayingPlaybackState(isPlaying: false)
          self.sendStateEvent("paused")
          self.sendTickEvent()
        }
      }
    }
  }
  #endif

  // MARK: - MPNowPlayingInfoCenter & MPRemoteCommandCenter

  private func ensureRemoteCommandsConfigured() {
    if remoteCommandsConfigured { return }
    remoteCommandsConfigured = true

    let cc = MPRemoteCommandCenter.shared()

    cc.playCommand.isEnabled = true
    cc.playCommand.addTarget { [weak self] _ in
      guard let self = self else { return .commandFailed }
      self.sendRemoteEvent(action: "play", positionMs: nil)
      return .success
    }

    cc.pauseCommand.isEnabled = true
    cc.pauseCommand.addTarget { [weak self] _ in
      guard let self = self else { return .commandFailed }
      self.sendRemoteEvent(action: "pause", positionMs: nil)
      return .success
    }

    cc.togglePlayPauseCommand.isEnabled = true
    cc.togglePlayPauseCommand.addTarget { [weak self] _ in
      guard let self = self else { return .commandFailed }
      self.sendRemoteEvent(action: self.isPlayingState ? "pause" : "play", positionMs: nil)
      return .success
    }

    cc.nextTrackCommand.isEnabled = true
    cc.nextTrackCommand.addTarget { [weak self] _ in
      guard let self = self else { return .commandFailed }
      self.sendRemoteEvent(action: "next", positionMs: nil)
      return .success
    }

    cc.previousTrackCommand.isEnabled = true
    cc.previousTrackCommand.addTarget { [weak self] _ in
      guard let self = self else { return .commandFailed }
      self.sendRemoteEvent(action: "prev", positionMs: nil)
      return .success
    }

    cc.changePlaybackPositionCommand.isEnabled = true
    cc.changePlaybackPositionCommand.addTarget { [weak self] event in
      guard
        let self = self,
        let posEvent = event as? MPChangePlaybackPositionCommandEvent
      else { return .commandFailed }
      let ms = max(0.0, posEvent.positionTime * 1000.0)
      self.performSeek(positionMs: ms)
      self.sendRemoteEvent(action: "seek", positionMs: ms)
      return .success
    }
  }

  private func applyNowPlayingInfo(metadata: TrackMetadataArgs, isPlaying: Bool) {
    var info = MPNowPlayingInfoCenter.default().nowPlayingInfo ?? [String: Any]()
    info[MPMediaItemPropertyTitle] = metadata.title
    info[MPMediaItemPropertyArtist] = metadata.artist
    info[MPMediaItemPropertyAlbumTitle] = metadata.album

    let durSec = currentEffectiveDurationMs() / 1000.0
    if durSec > 0 {
      info[MPMediaItemPropertyPlaybackDuration] = durSec
    }
    let elapsedSec = currentPositionSeconds()
    info[MPNowPlayingInfoPropertyElapsedPlaybackTime] = elapsedSec
    info[MPNowPlayingInfoPropertyPlaybackRate] = isPlaying ? Double(self.playbackRate) : 0.0

    #if os(iOS)
    if let artPath = metadata.artworkPath, !artPath.isEmpty,
       let image = UIImage(contentsOfFile: artPath) {
      let artwork = MPMediaItemArtwork(boundsSize: image.size) { _ in image }
      info[MPMediaItemPropertyArtwork] = artwork
    } else {
      info.removeValue(forKey: MPMediaItemPropertyArtwork)
    }
    #endif

    MPNowPlayingInfoCenter.default().nowPlayingInfo = info
  }

  private func updateNowPlayingPlaybackState(isPlaying: Bool, overrideElapsedSec: Double? = nil) {
    guard var info = MPNowPlayingInfoCenter.default().nowPlayingInfo else { return }
    let durSec = currentEffectiveDurationMs() / 1000.0
    if durSec > 0 {
      info[MPMediaItemPropertyPlaybackDuration] = durSec
    }
    info[MPNowPlayingInfoPropertyElapsedPlaybackTime] = overrideElapsedSec ?? currentPositionSeconds()
    info[MPNowPlayingInfoPropertyPlaybackRate] = isPlaying ? Double(self.playbackRate) : 0.0
    MPNowPlayingInfoCenter.default().nowPlayingInfo = info
  }

  private func currentPositionSeconds() -> Double {
    guard let t = player?.currentTime().seconds, t.isFinite && t >= 0 else {
      return 0.0
    }
    return t
  }

  private func currentEffectiveDurationMs() -> Double {
    if let itemDur = player?.currentItem?.duration.seconds, itemDur.isFinite && itemDur > 0 {
      return itemDur * 1000.0
    }
    return currentDurationMs
  }

  // MARK: - Channel Event Helpers

  private func sendTickEvent(overridePositionMs: Double? = nil) {
    guard let ch = eventChannel else { return }
    let posMs = overridePositionMs ?? (currentPositionSeconds() * 1000.0)
    let durMs = currentEffectiveDurationMs()
    let status = isPlayingState ? "playing" : (currentTrackId.isEmpty ? "stopped" : "paused")
    let payload = TickEvent(
      positionMs: posMs,
      durationMs: durMs,
      status: status,
      rate: isPlayingState ? Double(playbackRate) : 0.0,
      trackId: currentTrackId
    )
    try? ch.send(payload)
  }

  private func sendStateEvent(_ status: String) {
    guard let ch = eventChannel else { return }
    try? ch.send(StateEvent(status: status, trackId: currentTrackId))
  }

  private func sendEndedEvent(trackId: String) {
    guard let ch = eventChannel else { return }
    try? ch.send(EndedEvent(trackId: trackId))
  }

  private func sendRemoteEvent(action: String, positionMs: Double?) {
    guard let ch = eventChannel else { return }
    try? ch.send(RemoteEvent(action: action, positionMs: positionMs))
  }

  private func sendErrorEvent(_ message: String, trackId: String) {
    guard let ch = eventChannel else { return }
    try? ch.send(ErrorEvent(message: message, trackId: trackId))
  }
}

#if os(iOS)
extension CarminiumAudioPlugin: UIDocumentPickerDelegate {
  private static let audioExtensions: Set<String> = [
    "mp3", "flac", "m4a", "aac", "wav", "alac", "aif", "aiff", "ogg", "opus", "caf", "mp4"
  ]

  public func documentPicker(
    _ controller: UIDocumentPickerViewController,
    didPickDocumentsAt urls: [URL]
  ) {
    guard let invoke = pendingPickInvoke, let destDir = pendingPickDestination else { return }
    pendingPickInvoke = nil
    pendingPickDestination = nil

    DispatchQueue.global(qos: .userInitiated).async {
      var copied: [String] = []
      let fm = FileManager.default

      for url in urls {
        let accessed = url.startAccessingSecurityScopedResource()
        defer {
          if accessed { url.stopAccessingSecurityScopedResource() }
        }

        var isDir: ObjCBool = false
        if fm.fileExists(atPath: url.path, isDirectory: &isDir), isDir.boolValue {
          if let enumerator = fm.enumerator(
            at: url,
            includingPropertiesForKeys: [.isRegularFileKey],
            options: [.skipsHiddenFiles]
          ) {
            for case let fileURL as URL in enumerator {
              let ext = fileURL.pathExtension.lowercased()
              guard Self.audioExtensions.contains(ext) else { continue }
              if let dst = self.copyAudioFile(fileURL, into: destDir, fm: fm) {
                copied.append(dst.path)
              }
            }
          }
        } else {
          let ext = url.pathExtension.lowercased()
          if Self.audioExtensions.contains(ext) {
            if let dst = self.copyAudioFile(url, into: destDir, fm: fm) {
              copied.append(dst.path)
            }
          }
        }
      }

      DispatchQueue.main.async {
        invoke.resolve(PickFilesResult(copiedPaths: copied, canceled: false))
      }
    }
  }

  public func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
    guard let invoke = pendingPickInvoke else { return }
    pendingPickInvoke = nil
    pendingPickDestination = nil
    invoke.resolve(PickFilesResult(copiedPaths: [], canceled: true))
  }

  private func copyAudioFile(_ source: URL, into destDir: URL, fm: FileManager) -> URL? {
    let fileName = source.lastPathComponent
    var target = destDir.appendingPathComponent(fileName)
    if fm.fileExists(atPath: target.path) {
      let stem = source.deletingPathExtension().lastPathComponent
      let ext = source.pathExtension
      var idx = 1
      while fm.fileExists(atPath: target.path) {
        target = destDir.appendingPathComponent("\(stem) (\(idx)).\(ext)")
        idx += 1
      }
    }
    do {
      try fm.copyItem(at: source, to: target)
      return target
    } catch {
      NSLog("[CarminiumAudio] Failed copying \(source.path): \(error.localizedDescription)")
      return nil
    }
  }
}
#endif

@_cdecl("init_plugin_carminium_audio")
public func initPlugin() -> Plugin {
  return CarminiumAudioPlugin()
}

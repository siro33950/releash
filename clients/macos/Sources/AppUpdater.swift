import Foundation
import Sparkle

/// Sparkle によるアプリ自身の更新。更新の設定はこのアプリが所有し、サーバに置かない。
///
/// 自動更新は Sparkle が保存する設定を正とし、別に保存しない。nightly の受け入れはこのアプリの UserDefaults に保存する。
/// 更新が差し替えるのはアプリだけで、動いているサーバと workflow は止めない。
@MainActor @Observable final class AppUpdater: NSObject, SPUUpdaterDelegate {
  /// nightly の受け入れを保存する UserDefaults のキー。
  static let acceptsNightlyKey = "acceptsNightlyUpdates"
  /// nightly の項目が持つ `sparkle:channel` の値。
  static let nightlyChannel = "nightly"

  /// 見つかった新しい版（`sparkle:shortVersionString`）。無ければ nil。
  private(set) var availableVersion: String?
  /// 手動確認を始められるか。確認中は false になる。
  var canCheckForUpdates = false

  @ObservationIgnored private var controller: SPUStandardUpdaterController!
  private let defaults: UserDefaults
  private var observation: NSKeyValueObservation?

  /// Sparkle の updater を作る。`start()` を呼ぶまで appcast は読まない。
  init(defaults: UserDefaults = .standard) {
    self.defaults = defaults
    super.init()
    controller = SPUStandardUpdaterController(
      startingUpdater: false, updaterDelegate: self, userDriverDelegate: nil)
    observation = controller.updater.observe(\.canCheckForUpdates, options: [.new]) {
      [weak self] _, change in
      let enabled = change.newValue ?? false
      Task { @MainActor in self?.canCheckForUpdates = enabled }
    }
  }

  /// この bundle ID のアプリが更新を確認するかを決める。Debug のアプリ（`com.releash.macos.dev`）と bundle ID の無いものは確認しない。
  static func checksForUpdates(bundleIdentifier: String?) -> Bool {
    bundleIdentifier != nil && bundleIdentifier != "com.releash.macos.dev"
  }

  private var checksForUpdates: Bool {
    Self.checksForUpdates(bundleIdentifier: Bundle.main.bundleIdentifier)
  }

  /// updater を開始する。自動更新が ON なら Sparkle が確認とダウンロードを自動で行う。
  func start() {
    guard checksForUpdates else { return }
    controller.startUpdater()
  }

  /// 自動更新。ON は自動確認とダウンロードを行い、利用者がアプリを終了したときに適用する。初期値は ON。
  /// Sparkle の `automaticallyChecksForUpdates` と `automaticallyDownloadsUpdates` を正とする。
  var automaticUpdates: Bool {
    get {
      access(keyPath: \.automaticUpdates)
      return controller.updater.automaticallyChecksForUpdates
        && controller.updater.automaticallyDownloadsUpdates
    }
    set {
      withMutation(keyPath: \.automaticUpdates) {
        controller.updater.automaticallyChecksForUpdates = newValue
        controller.updater.automaticallyDownloadsUpdates = newValue
      }
    }
  }

  /// nightly の受け入れ。ON のときだけ nightly の項目が更新候補になる。初期値は OFF。
  /// 変えたら Sparkle に更新の周期をやり直させる。
  var acceptsNightly: Bool {
    get {
      access(keyPath: \.acceptsNightly)
      return defaults.bool(forKey: Self.acceptsNightlyKey)
    }
    set {
      withMutation(keyPath: \.acceptsNightly) {
        defaults.set(newValue, forKey: Self.acceptsNightlyKey)
      }
      availableVersion = nil
      controller.updater.resetUpdateCycleAfterShortDelay()
    }
  }

  /// Sparkle の手動確認を始め、結果を Sparkle の標準の画面で見せる。フッターの「更新」もこれを呼ぶ。
  func checkForUpdates() {
    guard checksForUpdates else { return }
    controller.checkForUpdates(nil)
  }

  /// 更新候補に含めるチャンネル。既定チャンネルは常に含まれ、`acceptsNightly` が ON のときだけ `nightly` を足す。
  nonisolated func allowedChannels(for updater: SPUUpdater) -> Set<String> {
    MainActor.assumeIsolated { acceptsNightly ? [Self.nightlyChannel] : [] }
  }

  /// 新しい版が見つかった。`availableVersion` にその版を置く。
  nonisolated func updater(_ updater: SPUUpdater, didFindValidUpdate item: SUAppcastItem) {
    MainActor.assumeIsolated { availableVersion = item.displayVersionString }
  }

  /// 新しい版が無かった。`availableVersion` を消す。
  nonisolated func updaterDidNotFindUpdate(_ updater: SPUUpdater, error: any Error) {
    MainActor.assumeIsolated { availableVersion = nil }
  }
}

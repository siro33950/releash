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
  private(set) var canCheckForUpdates = false

  private let controller: SPUStandardUpdaterController
  private let defaults: UserDefaults

  /// Sparkle の updater を作る。`start()` を呼ぶまで appcast は読まない。
  init(defaults: UserDefaults = .standard) {
    self.defaults = defaults
    controller = SPUStandardUpdaterController(
      startingUpdater: false, updaterDelegate: nil, userDriverDelegate: nil)
    super.init()
    fatalError("not implemented")
  }

  /// この bundle ID のアプリが更新を確認するかを決める。Debug のアプリ（`com.releash.macos.dev`）は確認しない。
  static func checksForUpdates(bundleIdentifier: String?) -> Bool {
    fatalError("not implemented")
  }

  /// updater を開始する。自動更新が ON なら Sparkle が確認とダウンロードを自動で行う。
  func start() {
    fatalError("not implemented")
  }

  /// 自動更新。ON は自動確認とダウンロードを行い、利用者がアプリを終了したときに適用する。初期値は ON。
  /// Sparkle の `automaticallyChecksForUpdates` と `automaticallyDownloadsUpdates` を正とする。
  var automaticUpdates: Bool {
    get { fatalError("not implemented") }
    set { fatalError("not implemented") }
  }

  /// nightly の受け入れ。ON のときだけ nightly の項目が更新候補になる。初期値は OFF。
  /// 変えたら Sparkle に更新の周期をやり直させる。
  var acceptsNightly: Bool {
    get { fatalError("not implemented") }
    set { fatalError("not implemented") }
  }

  /// Sparkle の手動確認を始め、結果を Sparkle の標準の画面で見せる。フッターの「更新」もこれを呼ぶ。
  func checkForUpdates() {
    fatalError("not implemented")
  }

  /// 更新候補に含めるチャンネル。既定チャンネルは常に含まれ、`acceptsNightly` が ON のときだけ `nightly` を足す。
  nonisolated func allowedChannels(for updater: SPUUpdater) -> Set<String> {
    fatalError("not implemented")
  }

  /// 新しい版が見つかった。`availableVersion` にその版を置く。
  nonisolated func updater(_ updater: SPUUpdater, didFindValidUpdate item: SUAppcastItem) {
    fatalError("not implemented")
  }

  /// 新しい版が無かった。`availableVersion` を消す。
  nonisolated func updaterDidNotFindUpdate(_ updater: SPUUpdater, error: any Error) {
    fatalError("not implemented")
  }
}

import Observation
import Sparkle
import XCTest

@testable import Releash

final class AppUpdaterTests: XCTestCase {
  @MainActor func test_自動更新_Sparkle側の変更を通知する() async throws {
    let app = AppUpdater()
    let updater = SPUStandardUpdaterController(
      startingUpdater: false, updaterDelegate: nil, userDriverDelegate: nil
    ).updater
    let checks = updater.automaticallyChecksForUpdates
    let downloads = updater.automaticallyDownloadsUpdates
    defer {
      updater.automaticallyChecksForUpdates = checks
      updater.automaticallyDownloadsUpdates = downloads
    }
    for keyPath in [\SPUUpdater.automaticallyChecksForUpdates, \.automaticallyDownloadsUpdates] {
      app.automaticUpdates = true
      let changed = expectation(description: "automaticUpdates changed")
      withObservationTracking {
        _ = app.automaticUpdates
      } onChange: {
        changed.fulfill()
      }
      updater[keyPath: keyPath] = false
      await fulfillment(of: [changed], timeout: 1)
      XCTAssertFalse(app.automaticUpdates)
    }
  }

  @MainActor func test_更新通知_nightly設定変更でstableを保持しnightlyだけ除外する() throws {
    let suite = "AppUpdaterTests.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let app = AppUpdater(defaults: defaults)
    let updater = SPUStandardUpdaterController(
      startingUpdater: false, updaterDelegate: nil, userDriverDelegate: nil
    ).updater
    for nightly in [false, true] {
      app.acceptsNightly = true
      var dictionary: [String: Any] = [
        "enclosure": [
          "url": "https://example.invalid/update.zip", "sparkle:version": "42",
          "sparkle:shortVersionString": "0.4.21",
        ]
      ]
      if nightly { dictionary["sparkle:channel"] = AppUpdater.nightlyChannel }
      let item = try XCTUnwrap(SUAppcastItem(dictionary: dictionary))
      app.updater(updater, didFindValidUpdate: item)
      app.acceptsNightly = false
      XCTAssertEqual(app.availableVersion, nightly ? nil : "0.4.21")
      app.updater(updater, didFindValidUpdate: item)
      XCTAssertEqual(app.availableVersion, nightly ? nil : "0.4.21")
    }
  }

  @MainActor func test_更新対象_本番アプリは確認できる() {
    // Given / When / Then
    XCTAssertTrue(AppUpdater.checksForUpdates(bundleIdentifier: "com.releash.macos"))
  }

  @MainActor func test_更新対象_bundleIDが無い場合は確認しない() {
    // Given
    let bundleIdentifier: String? = nil
    // When
    let checks = AppUpdater.checksForUpdates(bundleIdentifier: bundleIdentifier)
    // Then
    XCTAssertFalse(checks)
  }

  @MainActor func test_nightly設定_初期値と切り替えと再生成後の保持() throws {
    // Given
    let suite = "AppUpdaterTests.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let app = AppUpdater(defaults: defaults)
    let controller = SPUStandardUpdaterController(
      startingUpdater: false, updaterDelegate: nil, userDriverDelegate: nil)
    // When / Then
    XCTAssertFalse(app.acceptsNightly)
    XCTAssertEqual(app.allowedChannels(for: controller.updater), [])
    XCTAssertNil(app.availableVersion)
    XCTAssertFalse(app.canCheckForUpdates)
    for enabled in [true, false] {
      app.acceptsNightly = enabled
      XCTAssertEqual(app.acceptsNightly, enabled)
      XCTAssertEqual(defaults.bool(forKey: AppUpdater.acceptsNightlyKey), enabled)
      XCTAssertEqual(app.allowedChannels(for: controller.updater), enabled ? ["nightly"] : [])
      XCTAssertEqual(AppUpdater(defaults: defaults).acceptsNightly, enabled)
    }
  }

  @MainActor func test_自動更新_Sparkleの確認とダウンロードの設定を同期する() throws {
    // Given
    let suite = "AppUpdaterTests.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let app = AppUpdater(defaults: defaults)
    let controller = SPUStandardUpdaterController(
      startingUpdater: false, updaterDelegate: nil, userDriverDelegate: nil)
    let checks = controller.updater.automaticallyChecksForUpdates
    let downloads = controller.updater.automaticallyDownloadsUpdates
    defer {
      controller.updater.automaticallyChecksForUpdates = checks
      controller.updater.automaticallyDownloadsUpdates = downloads
    }
    // When / Then
    for enabled in [false, true] {
      app.automaticUpdates = enabled
      XCTAssertEqual(app.automaticUpdates, enabled)
      XCTAssertEqual(controller.updater.automaticallyChecksForUpdates, enabled)
      XCTAssertEqual(controller.updater.automaticallyDownloadsUpdates, enabled)
      XCTAssertEqual(AppUpdater(defaults: defaults).automaticUpdates, enabled)
    }
  }
  @MainActor func test_更新通知_発見した版を保持し更新なしで消す() async throws {
    // Given
    let suite = "AppUpdaterTests.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let app = AppUpdater(defaults: defaults)
    let controller = SPUStandardUpdaterController(
      startingUpdater: false, updaterDelegate: nil, userDriverDelegate: nil)
    let item = try XCTUnwrap(
      SUAppcastItem(dictionary: [
        "enclosure": [
          "url": "https://example.invalid/update.zip", "sparkle:version": "42",
          "sparkle:shortVersionString": "0.4.21",
        ]
      ]))
    // When
    app.updater(controller.updater, didFindValidUpdate: item)
    await Task.yield()
    // Then
    XCTAssertEqual(app.availableVersion, "0.4.21")
    // When
    app.updaterDidNotFindUpdate(controller.updater, error: NSError(domain: "test", code: 1))
    await Task.yield()
    // Then
    XCTAssertNil(app.availableVersion)
  }
}

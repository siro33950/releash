import AppKit
import Sparkle
import XCTest

@testable import Releash

final class UpdateUIAcceptanceTests: XCTestCase {
  @MainActor func test_b045_自動ダウンロード中_更新ボタンは無効で完了後に押せる() async throws {
    // Given
    let model = AppModel()
    let delegate: any SPUUpdaterDelegate = model.updater
    let controller = SPUStandardUpdaterController(
      startingUpdater: false, updaterDelegate: delegate, userDriverDelegate: nil)
    let checks = controller.updater.automaticallyChecksForUpdates
    let downloads = controller.updater.automaticallyDownloadsUpdates
    defer {
      controller.updater.automaticallyChecksForUpdates = checks
      controller.updater.automaticallyDownloadsUpdates = downloads
    }
    model.updater.automaticUpdates = true
    let url = try XCTUnwrap(URL(string: "https://example.invalid/update.zip"))
    let item = try XCTUnwrap(
      SUAppcastItem(dictionary: [
        "enclosure": [
          "url": url.absoluteString, "sparkle:version": "0.4.21",
          "sparkle:shortVersionString": "0.4.21",
        ]
      ]))
    model.updater.updater(controller.updater, didFindValidUpdate: item)
    delegate.updater?(
      controller.updater, willDownloadUpdate: item, with: NSMutableURLRequest(url: url))
    // When
    let host = await ViewHarness(FooterView(model: model))
    defer { host.close() }
    // Then
    try await eventuallyView(host) {
      host.element("0.4.21") != nil && host.element("更新")?.enabled == false
    }
    // When
    delegate.updater?(controller.updater, didDownloadUpdate: item)
    // Then
    try await eventuallyView(host) {
      host.element("0.4.21") != nil && host.element("更新")?.enabled == true
    }
  }

  @MainActor func test_b007_配布名_メニューバーのtooltipとAboutと終了がReleashになる() throws {
    // Given
    let status = StatusMenu(model: AppModel()) {}
    defer { status.remove() }
    let menu = try XCTUnwrap(status.item.menu)
    // When
    status.menuNeedsUpdate(menu)
    // Then
    XCTAssertEqual(status.item.button?.toolTip, "Releash")
    XCTAssertTrue(menu.items.contains { $0.title == "Releash について" })
    XCTAssertTrue(menu.items.contains { $0.title == "Releash を終了" })
  }

  @MainActor func test_b013_サーバ未接続_Systemの更新設定を切り替えられる() async throws {
    // Given
    let host = await ViewHarness(SystemSettingsView(model: AppModel()))
    defer { host.close() }
    try await eventuallyView(host) {
      host.element("自動更新") != nil && host.element("nightly アップデートを受け入れる") != nil
    }
    for label in ["自動更新", "nightly アップデートを受け入れる"] {
      let original = try XCTUnwrap(host.element(label))
      XCTAssertTrue(original.enabled)
      // When
      try await host.press(label)
      // Then
      try await eventuallyView(host) { host.element(label)?.value != original.value }
      // When
      try await host.press(label)
      // Then
      try await eventuallyView(host) { host.element(label)?.value == original.value }
    }
  }
}

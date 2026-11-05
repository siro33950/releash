import AppKit
import XCTest

@testable import Releash

final class UpdateUIAcceptanceTests: XCTestCase {
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

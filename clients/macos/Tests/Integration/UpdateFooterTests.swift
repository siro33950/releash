import Sparkle
import XCTest

@testable import Releash

final class UpdateFooterTests: XCTestCase {
  @MainActor func test_更新通知_発見した版と更新ボタンを表示し更新なしで消す() async throws {
    // Given
    let model = AppModel()
    let host = await ViewHarness(FooterView(model: model))
    defer { host.close() }
    let controller = SPUStandardUpdaterController(
      startingUpdater: false, updaterDelegate: nil, userDriverDelegate: nil)
    let item = try XCTUnwrap(
      SUAppcastItem(dictionary: [
        "enclosure": [
          "url": "https://example.invalid/update.zip", "sparkle:version": "0.4.21",
          "sparkle:shortVersionString": "0.4.21",
        ]
      ]))
    XCTAssertNil(host.element("更新"))
    // When
    model.updater.updater(controller.updater, didFindValidUpdate: item)
    // Then
    try await eventuallyView(host) {
      host.element("0.4.21") != nil && host.element("更新") != nil
    }
    // When
    model.updater.updaterDidNotFindUpdate(
      controller.updater, error: NSError(domain: "test", code: 1))
    // Then
    try await eventuallyView(host) { host.element("更新") == nil }
  }
}

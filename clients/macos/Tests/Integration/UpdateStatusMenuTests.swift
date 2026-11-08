import XCTest

@testable import Releash

final class UpdateStatusMenuTests: XCTestCase {
  @MainActor func test_更新確認メニュー_確認可否に応じて有効状態が変わる() throws {
    // Given
    let model = AppModel()
    let status = StatusMenu(model: model) {}
    defer { status.remove() }
    let menu = try XCTUnwrap(status.item.menu)
    for enabled in [false, true, false] {
      // When
      model.updater.canCheckForUpdates = enabled
      status.menuNeedsUpdate(menu)
      // Then
      XCTAssertEqual(try XCTUnwrap(menu.items.first { $0.title == "アップデートを確認" }).isEnabled, enabled)
    }
  }
}

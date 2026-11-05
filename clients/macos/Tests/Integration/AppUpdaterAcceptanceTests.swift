import Sparkle
import XCTest

@testable import Releash

final class AppUpdaterAcceptanceTests: XCTestCase {
  @MainActor func test_b012_b013_更新設定_サーバなしでnightlyの設定を保存して再生成後も保持する() throws {
    // Given
    let suite = "AppUpdaterAcceptanceTests.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    var app: AppUpdater? = AppUpdater(defaults: defaults)
    XCTAssertFalse(try XCTUnwrap(app).acceptsNightly)
    // When
    try XCTUnwrap(app).acceptsNightly = true
    app = nil
    app = AppUpdater(defaults: try XCTUnwrap(UserDefaults(suiteName: suite)))
    // Then
    XCTAssertTrue(try XCTUnwrap(app).acceptsNightly)
    // When
    try XCTUnwrap(app).acceptsNightly = false
    app = nil
    app = AppUpdater(defaults: try XCTUnwrap(UserDefaults(suiteName: suite)))
    // Then
    XCTAssertFalse(try XCTUnwrap(app).acceptsNightly)
  }

  @MainActor func test_b015_b016_nightly受け入れ_設定に応じて候補のチャンネルが変わる() throws {
    // Given
    let suite = "AppUpdaterAcceptanceTests.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let app = AppUpdater(defaults: defaults)
    let controller = SPUStandardUpdaterController(
      startingUpdater: false, updaterDelegate: nil, userDriverDelegate: nil)
    // When / Then
    XCTAssertEqual(app.allowedChannels(for: controller.updater), [])
    app.acceptsNightly = true
    XCTAssertEqual(app.allowedChannels(for: controller.updater), ["nightly"])
    app.acceptsNightly = false
    XCTAssertEqual(app.allowedChannels(for: controller.updater), [])
  }
}

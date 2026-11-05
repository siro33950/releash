import XCTest

@testable import Releash

final class AppUpdaterAcceptanceTests: XCTestCase {
  @MainActor func test_b019_Debugアプリ_更新の確認を許可しない() {
    // Given
    let bundleIdentifier = "com.releash.macos.dev"
    // When
    let checks = AppUpdater.checksForUpdates(bundleIdentifier: bundleIdentifier)
    // Then
    XCTAssertFalse(checks)
  }
}

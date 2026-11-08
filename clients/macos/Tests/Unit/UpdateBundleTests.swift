import XCTest

@testable import Releash

final class UpdateBundleTests: XCTestCase {
  @MainActor func test_更新対象_Debug以外の隔離bundleは確認できる() {
    // Given
    let identifier = "test.releash.update.\(UUID().uuidString)"
    // When
    let allowed = AppUpdater.checksForUpdates(bundleIdentifier: identifier)
    // Then
    XCTAssertTrue(allowed)
  }
}

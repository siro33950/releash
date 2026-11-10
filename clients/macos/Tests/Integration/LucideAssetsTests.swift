import AppKit
import XCTest

@testable import Releash

final class LucideAssetsTests: XCTestCase {
  @MainActor func test_Lucide画像_全画像をテンプレートとして読み込める() throws {
    // Given
    let bundle = Bundle(for: AppModel.self)
    let names = [
      "bot", "waypoints", "terminal", "git-fork", "folder-plus", "refresh-cw", "plus", "house",
      "git-branch", "circle-check", "git-pull-request", "git-pull-request-draft", "git-merge",
      "git-pull-request-closed", "list", "file", "message-square", "panel-right", "history",
      "ellipsis", "diff", "square-arrow-out-up-right", "unfold-vertical", "git-commit-horizontal",
      "rows-2", "columns-2", "triangle-alert", "sparkles", "server", "settings", "folder",
      "monitor",
    ]
    for name in names {
      // When
      let image = try XCTUnwrap(bundle.image(forResource: name), name)
      // Then
      XCTAssertTrue(image.isTemplate, name)
      XCTAssertTrue(image.isValid, name)
    }
  }
}

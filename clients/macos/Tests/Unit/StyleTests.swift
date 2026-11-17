import SwiftUI
import XCTest

@testable import Releash

final class StyleTests: XCTestCase {
  func test_Check表示_bucketに対応するアイコンと色を返す() {
    // Given
    let cases: [(Releash_Client_V1_PullRequestCheckBucket, String, Color)] = [
      (.fail, "circle-alert", .red),
      (.cancel, "triangle-alert", .orange),
      (.pending, "refresh-cw", .yellow),
      (.unspecified, "refresh-cw", .yellow),
      (.UNRECOGNIZED(99), "refresh-cw", .yellow),
    ]
    for (bucket, icon, color) in cases {
      // When
      let appearance = UIStyle.checkAppearance(bucket)
      // Then
      XCTAssertEqual(appearance.icon, icon)
      XCTAssertEqual(appearance.color, color)
    }
  }
}

import AppKit
import ApplicationServices
import SwiftUI
import XCTest

@testable import Releash

final class WorktreeWarningTests: XCTestCase {
  @MainActor func test_リポジトリ警告_幅を変更しても見出しの右端に追随する() async throws {
    // Given
    let model = AppModel()
    model.repositories = [
      .with {
        $0.path = "/repo"
        $0.status.error = "Repository failed"
      }
    ]
    let host = await ViewHarness(
      GeometryReader { geometry in
        SidebarView(
          model: model, creating: .constant(false), sidebarWidth: geometry.size.width)
      },
      size: NSSize(width: 280, height: 700))
    defer { host.close() }
    let content = try XCTUnwrap(host.window.contentView)
    func warningFrame(_ view: NSView) -> NSRect? {
      if view.toolTip == "Repository failed" { return view.convert(view.bounds, to: content) }
      return view.subviews.lazy.compactMap { warningFrame($0) }.first
    }
    try await eventually { warningFrame(content) != nil }
    let initial = try XCTUnwrap(warningFrame(content))
    let trailing = content.bounds.maxX - initial.maxX

    // When
    host.window.setContentSize(NSSize(width: 400, height: 700))

    // Then
    try await eventually {
      guard let frame = warningFrame(content) else { return false }
      return abs(frame.maxX - initial.maxX - 120) < 1
    }
    let resized = try XCTUnwrap(warningFrame(content))
    XCTAssertEqual(content.bounds.maxX - resized.maxX, trailing, accuracy: 1)
    XCTAssertGreaterThan(resized.minX, content.bounds.midX)
    XCTAssertTrue(content.bounds.contains(resized))
  }
}

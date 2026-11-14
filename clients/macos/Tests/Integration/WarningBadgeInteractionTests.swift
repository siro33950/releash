import AppKit
import SwiftUI
import XCTest

@testable import Releash

final class WarningBadgeInteractionTests: XCTestCase {
  @MainActor func test_警告バッジ_クリックしてworktreeを選択できる() async throws {
    // Given
    let model = AppModel()
    model.repositories = [
      .with {
        $0.path = "/warning"
        $0.branches.items = [
          .with {
            $0.name = "feature"
            $0.worktreePath = "/warning/feature"
            $0.dirtyCountError = "Unavailable"
          }
        ]
      }
    ]
    let host = await ViewHarness(SidebarView(model: model, creating: .constant(false)))
    defer { host.close() }
    func tooltip(in view: NSView) -> NSView? {
      if view.toolTip == "Changes unavailable: Unavailable" { return view }
      return view.subviews.lazy.compactMap { tooltip(in: $0) }.first
    }
    let content = try XCTUnwrap(host.window.contentView)
    try await eventuallyView(host) { tooltip(in: content) != nil }
    let badge = try XCTUnwrap(tooltip(in: content))
    let point = badge.convert(NSPoint(x: badge.bounds.midX, y: badge.bounds.midY), to: nil)

    // When
    for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
      NSApp.postEvent(
        try XCTUnwrap(
          NSEvent.mouseEvent(
            with: type, location: point, modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: host.window.windowNumber, context: nil, eventNumber: 1,
            clickCount: 1, pressure: 0)), atStart: false)
    }

    // Then
    try await eventuallyView(host) { model.selectedWorktree == "/warning/feature" }
    await model.stop()
  }
}

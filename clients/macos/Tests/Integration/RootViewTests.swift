import AppKit
import XCTest

@testable import Releash

final class RootViewTests: XCTestCase {
  @MainActor func test_作成ボタン_中央クリックで作成画面を開く() async throws {
    // Given
    let model = try await testModel(TestHTTPClient())
    let host = await ViewHarness(RootView(model: model))
    defer { host.close() }
    try await eventuallyView(host) { host.element("Worktreeを作成") != nil }
    let content = try XCTUnwrap(host.window.contentView)
    let inset = UIStyle.windowPadding + UIStyle.tabBarHeight / 2
    let point = content.convert(
      NSPoint(
        x: UIStyle.windowPadding + UIStyle.sidebarWidth - UIStyle.spacing
          - UIStyle.iconButtonWidth / 2,
        y: content.isFlipped ? inset : content.bounds.height - inset), to: nil)
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
    try await eventuallyView(host) { host.element("新しいWorktree") != nil }
    await model.stop()
  }

  @MainActor func testColumnDividersAllowWideRightPanelAndKeepCenterAndSidebarLimits() async throws
  {
    let model = try await testModel(TestHTTPClient())
    let host = await ViewHarness(
      RootView(model: model), size: NSSize(width: UIStyle.windowWidth, height: UIStyle.windowHeight)
    )
    defer { host.close() }
    func splits(_ view: NSView) -> [NSSplitView] {
      ((view as? NSSplitView).map { [$0] } ?? []) + view.subviews.flatMap(splits)
    }
    let content = try XCTUnwrap(host.window.contentView)
    try await eventuallyView(host) {
      content.layoutSubtreeIfNeeded()
      return splits(content).count == 2
    }
    let outer = try XCTUnwrap(splits(content).first)
    let inner = try XCTUnwrap(splits(content).last)
    XCTAssertTrue(outer.isVertical)
    XCTAssertTrue(inner.isVertical)
    XCTAssertEqual(outer.arrangedSubviews.count, 2)
    XCTAssertEqual(inner.arrangedSubviews.count, 2)
    let initialColumnWidth =
      (host.window.contentLayoutRect.width - UIStyle.windowPadding * 2 - 280) / 2
    try await eventuallyView(host) {
      content.layoutSubtreeIfNeeded()
      return abs(outer.arrangedSubviews[0].frame.width - 280) < 0.5
        && abs(inner.arrangedSubviews[0].frame.width - initialColumnWidth) < 0.5
        && abs(inner.arrangedSubviews[1].frame.width - initialColumnWidth) < 0.5
    }
    XCTAssertEqual(outer.arrangedSubviews[0].frame.width, 280, accuracy: 0.5)
    XCTAssertEqual(inner.arrangedSubviews[0].frame.width, initialColumnWidth, accuracy: 0.5)
    XCTAssertEqual(inner.arrangedSubviews[1].frame.width, initialColumnWidth, accuracy: 0.5)
    host.window.setContentSize(NSSize(width: 1600, height: 900))
    content.layoutSubtreeIfNeeded()
    inner.setPosition(inner.bounds.width - 560, ofDividerAt: 0)
    try await eventuallyView(host) {
      content.layoutSubtreeIfNeeded()
      return inner.arrangedSubviews[1].frame.width > 400
        && inner.arrangedSubviews[0].frame.width >= 220
    }
    let expandedRightWidth = inner.arrangedSubviews[1].frame.width
    try await host.press("右パネルを閉じる")
    try await eventuallyView(host) { abs(inner.arrangedSubviews[1].frame.width - 36) < 0.5 }
    try await host.press("右パネルを開く")
    try await eventuallyView(host) {
      abs(inner.arrangedSubviews[1].frame.width - expandedRightWidth) < 0.5
    }
    outer.setPosition(600, ofDividerAt: 0)
    try await eventuallyView(host) {
      content.layoutSubtreeIfNeeded()
      return abs(outer.arrangedSubviews[0].frame.width - 400) < 1
    }
    XCTAssertLessThanOrEqual(outer.arrangedSubviews[0].frame.width, 400)
    inner.setPosition(50, ofDividerAt: 0)
    try await eventuallyView(host) {
      content.layoutSubtreeIfNeeded()
      return abs(inner.arrangedSubviews[0].frame.width - 220) < 1
    }
    XCTAssertGreaterThanOrEqual(inner.arrangedSubviews[0].frame.width, 220)
    XCTAssertGreaterThan(inner.arrangedSubviews[1].frame.width, 400)
    XCTAssertLessThanOrEqual(outer.arrangedSubviews[0].frame.width, 400)
    inner.setPosition(inner.bounds.width - 50, ofDividerAt: 0)
    try await eventuallyView(host) {
      content.layoutSubtreeIfNeeded()
      return abs(inner.arrangedSubviews[1].frame.width - 220) < 1
    }
    host.window.setContentSize(
      NSSize(width: UIStyle.windowMinimumWidth + UIStyle.windowPadding * 2, height: 900))
    try await eventuallyView(host) {
      content.layoutSubtreeIfNeeded()
      return abs(outer.arrangedSubviews[0].frame.width - 280) < 0.5
        && abs(inner.arrangedSubviews[0].frame.width - 220) < 0.5
        && abs(inner.arrangedSubviews[1].frame.width - 220) < 0.5
    }
    try await host.press("右パネルを閉じる")
    try await eventuallyView(host) { abs(inner.arrangedSubviews[1].frame.width - 36) < 0.5 }
    outer.setPosition(400, ofDividerAt: 0)
    try await eventuallyView(host) {
      content.layoutSubtreeIfNeeded()
      return abs(outer.arrangedSubviews[0].frame.width - 400) < 0.5
    }
    try await host.press("右パネルを開く")
    try await eventuallyView(host) {
      content.layoutSubtreeIfNeeded()
      return abs(outer.arrangedSubviews[0].frame.width - 280) < 0.5
        && inner.arrangedSubviews[0].frame.width >= UIStyle.centerMinimumWidth
        && inner.arrangedSubviews[1].frame.width >= UIStyle.rightPanelMinimumWidth
    }
    await model.stop()
  }
}

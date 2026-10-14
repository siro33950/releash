import AppKit
import XCTest

@testable import Releash

final class RootViewTests: XCTestCase {
  @MainActor func testColumnDividersAllowWideRightPanelAndKeepCenterAndSidebarLimits() async throws {
    let model = try await testModel(TestHTTPClient())
    let host = await ViewHarness(RootView(model: model))
    defer { host.close() }
    host.window.setContentSize(NSSize(width: 1600, height: 900))
    func splits(_ view: NSView) -> [NSSplitView] {
      ((view as? NSSplitView).map { [$0] } ?? []) + view.subviews.flatMap(splits)
    }
    let content = try XCTUnwrap(host.window.contentView)
    try await eventuallyView(host) { content.layoutSubtreeIfNeeded(); return splits(content).count == 2 }
    let outer = try XCTUnwrap(splits(content).first)
    let inner = try XCTUnwrap(splits(content).last)
    XCTAssertTrue(outer.isVertical); XCTAssertTrue(inner.isVertical)
    XCTAssertEqual(outer.arrangedSubviews.count, 2); XCTAssertEqual(inner.arrangedSubviews.count, 2)
    inner.setPosition(inner.bounds.width - 560, ofDividerAt: 0)
    try await eventuallyView(host) {
      content.layoutSubtreeIfNeeded()
      return inner.arrangedSubviews[1].frame.width > 400 && inner.arrangedSubviews[0].frame.width >= UIStyle.centerMinimumWidth
    }
    outer.setPosition(600, ofDividerAt: 0)
    try await eventuallyView(host) {
      content.layoutSubtreeIfNeeded()
      return abs(outer.arrangedSubviews[0].frame.width - UIStyle.sidebarMaximumWidth) < 1
    }
    XCTAssertLessThanOrEqual(outer.arrangedSubviews[0].frame.width, 400)
    inner.setPosition(50, ofDividerAt: 0)
    try await eventuallyView(host) {
      content.layoutSubtreeIfNeeded()
      return abs(inner.arrangedSubviews[0].frame.width - UIStyle.centerMinimumWidth) < 1
    }
    XCTAssertGreaterThanOrEqual(inner.arrangedSubviews[0].frame.width, UIStyle.centerMinimumWidth)
    XCTAssertGreaterThan(inner.arrangedSubviews[1].frame.width, 400)
    XCTAssertLessThanOrEqual(outer.arrangedSubviews[0].frame.width, 400)
    await model.stop()
  }
}

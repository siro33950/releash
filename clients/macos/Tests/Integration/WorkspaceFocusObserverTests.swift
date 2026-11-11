import AppKit
import SwiftUI
import XCTest

@testable import Releash

final class WorkspaceFocusObserverTests: XCTestCase {
  @MainActor func test_フォーカス_子ビューのクリックを妨げず操作領域を切り替える() async throws {
    // Given
    var focused = ""
    var clicks = 0
    let host = await ViewHarness(
      HStack(spacing: 0) {
        Button {
          clicks += 1
        } label: {
          Color.red.frame(width: 200, height: 200)
        }.buttonStyle(.plain)
          .background(WorkspaceFocusObserver { focused = "center" })
        Button {
          clicks += 1
        } label: {
          Color.blue.frame(width: 200, height: 200)
        }.buttonStyle(.plain)
          .background(WorkspaceFocusObserver { focused = "files" })
      }, size: NSSize(width: 400, height: 200))
    defer { host.close() }
    for (index, expected) in ["center", "files", "center"].enumerated() {
      // When
      let local = NSPoint(x: expected == "center" ? 100 : 300, y: 100)
      for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
        let event = try XCTUnwrap(
          NSEvent.mouseEvent(
            with: type, location: local, modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: host.window.windowNumber, context: nil, eventNumber: index, clickCount: 1,
            pressure: type == .leftMouseDown ? 1 : 0))
        NSApp.postEvent(event, atStart: false)
        try await Task.sleep(for: .milliseconds(30))
      }
      // Then
      let deadline = ContinuousClock.now.advanced(by: .seconds(3))
      while clicks < index + 1 && ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(20))
      }
      XCTAssertEqual(focused, expected)
      XCTAssertEqual(clicks, index + 1)
    }
  }

  @MainActor func test_ファイル境界_左右へドラッグすると両領域の幅が変わる() async throws {
    // Given
    let host = await ViewHarness(
      WorkspaceColumnSplit(
        minimumLeadingWidth: 180, minimumTrailingWidth: 240, initialLeadingWidth: 260,
        dividerThickness: 1
      ) {
        Color.red
      } trailing: {
        Color.blue
      }, size: NSSize(width: 1000, height: 700))
    defer { host.close() }
    func findSplit(_ view: NSView) -> NSSplitView? {
      if let split = view as? NSSplitView { return split }
      return view.subviews.lazy.compactMap(findSplit).first
    }
    let split = try XCTUnwrap(findSplit(try XCTUnwrap(host.window.contentView)))
    for offset in [60.0, -60.0] {
      let left = split.arrangedSubviews[0].frame.width
      let right = split.arrangedSubviews[1].frame.width
      let start = split.convert(NSPoint(x: left, y: split.bounds.midY), to: nil)
      // When
      for (type, delta) in [
        (NSEvent.EventType.leftMouseDown, 0.0), (.leftMouseDragged, offset), (.leftMouseUp, offset),
      ] {
        let event = try XCTUnwrap(
          NSEvent.mouseEvent(
            with: type, location: NSPoint(x: start.x + delta, y: start.y), modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: host.window.windowNumber,
            context: nil, eventNumber: 0, clickCount: 1, pressure: 1))
        NSApp.postEvent(event, atStart: false)
      }
      let deadline = ContinuousClock.now.advanced(by: .seconds(3))
      while split.arrangedSubviews[0].frame.width == left && ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(20))
      }
      // Then
      XCTAssertGreaterThan((split.arrangedSubviews[0].frame.width - left) * offset, 0)
      XCTAssertLessThan((split.arrangedSubviews[1].frame.width - right) * offset, 0)
    }
  }

  @MainActor func test_ファイルタブ_クリックで選択と操作領域を切り替える() async throws {
    // Given
    let model = AppModel()
    model.filePanes.openFile("first.rs", base: "head")
    model.filePanes.openFile("second.rs", base: "head")
    let pane = try XCTUnwrap(model.filePanes.controller.focusedPaneId)
    let host = await ViewHarness(FileWorkspaceView(model: model, worktree: "/repo"))
    defer { host.close() }
    // When
    for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
      let event = try XCTUnwrap(
        NSEvent.mouseEvent(
          with: type, location: NSPoint(x: 310, y: 684), modifierFlags: [],
          timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: host.window.windowNumber,
          context: nil, eventNumber: 0, clickCount: 1, pressure: 1))
      NSApp.postEvent(event, atStart: false)
      try await Task.sleep(for: .milliseconds(30))
    }
    // Then
    let deadline = ContinuousClock.now.advanced(by: .seconds(3))
    while model.filePanes.controller.selectedTab(inPane: pane)?.title != "first.rs"
      && ContinuousClock.now < deadline
    {
      try await Task.sleep(for: .milliseconds(20))
    }
    XCTAssertEqual(model.filePanes.controller.selectedTab(inPane: pane)?.title, "first.rs")
    XCTAssertTrue(model.filesFocused)
  }

}

import AppKit
import ApplicationServices
import SwiftUI
import Vision
import XCTest

@testable import Releash

final class WorktreeWarningTests: XCTestCase {
  @MainActor func test_警告のホバー_更新後の全エラーを表示してカード選択を妨げない() async throws {
    // Given
    let model = AppModel()
    model.repositories = [
      .with {
        $0.path = "/repo"
        $0.branches.items = [
          .with {
            $0.name = "feature"
            $0.worktreePath = "/repo/feature"
            $0.dirtyCountError = "Old failure"
          }
        ]
        $0.worktrees.items = [.with { $0.path = "/repo/feature" }]
      }
    ]
    let host = await ViewHarness(
      SidebarView(model: model, creating: .constant(false)),
      size: NSSize(width: 340, height: 700))
    defer { host.close() }
    let content = try XCTUnwrap(host.window.contentView)
    func warning(_ view: NSView) -> NSView? {
      if view.toolTip?.contains("Changes unavailable") == true { return view }
      return view.subviews.lazy.compactMap { warning($0) }.first
    }
    try await eventually { warning(content) != nil }

    // When
    model.repositories[0].branches.items[0].dirtyCountError = "Dirty failed"
    model.repositories[0].branches.items[0].pullRequestError = "PR failed"
    model.repositories[0].worktrees.items[0].status.error = "Worktree failed"
    try await eventually { warning(content)?.toolTip?.contains("Worktree failed") == true }
    let target = try XCTUnwrap(warning(content))
    let bounds = host.window.convertToScreen(target.convert(target.bounds, to: nil))
    let desktop = try XCTUnwrap(NSScreen.screens.first).frame
    let point = CGPoint(x: bounds.midX, y: desktop.height - bounds.midY)
    let previous = try XCTUnwrap(CGEvent(source: nil)?.location)
    func move(_ point: CGPoint) throws {
      XCTAssertEqual(CGWarpMouseCursorPosition(point), .success)
      let event = try XCTUnwrap(
        CGEvent(
          mouseEventSource: nil, mouseType: .mouseMoved,
          mouseCursorPosition: point, mouseButton: .left))
      event.post(tap: .cghidEventTap)
      let localEvent = try XCTUnwrap(
        NSEvent.mouseEvent(
          with: .mouseMoved,
          location: host.window.convertPoint(
            fromScreen:
              NSPoint(x: point.x, y: desktop.height - point.y)),
          modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
          windowNumber: host.window.windowNumber, context: nil, eventNumber: 1,
          clickCount: 0, pressure: 0))
      NSApp.postEvent(localEvent, atStart: false)
    }
    defer { try? move(previous) }
    let app = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
    XCTAssertEqual(
      AXUIElementSetAttributeValue(app, kAXFrontmostAttribute as CFString, kCFBooleanTrue), .success
    )
    host.window.makeKeyAndOrderFront(nil)
    try await eventually { NSApp.isActive && host.window.isKeyWindow }
    try move(CGPoint(x: point.x - 40, y: point.y))
    try await Task.sleep(for: .milliseconds(100))
    try move(point)

    // Then
    func displayedText() throws -> String? {
      guard
        let panel = NSApp.windows.first(where: {
          $0 !== host.window && $0.isVisible && String(describing: type(of: $0)) == "NSToolTipPanel"
        }), let view = panel.contentView
      else { return nil }
      let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
      view.cacheDisplay(in: view.bounds, to: bitmap)
      let request = VNRecognizeTextRequest()
      request.recognitionLanguages = ["en-US"]
      request.usesLanguageCorrection = false
      try VNImageRequestHandler(cgImage: XCTUnwrap(bitmap.cgImage)).perform([request])
      return (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
        .joined(separator: " ")
    }
    var text: String?
    try await eventually {
      text = try? displayedText()
      return text != nil
    }
    let actual = try XCTUnwrap(text)
    for expected in [
      "Changes unavailable", "Dirty failed", "PR unavailable", "PR failed", "Worktree failed",
    ] {
      XCTAssertTrue(actual.contains(expected), actual)
    }
    XCTAssertFalse(actual.contains("Old failure"))
    try await host.press("worktree:feature")
    XCTAssertEqual(model.selectedWorktree, "/repo/feature")
    try move(CGPoint(x: point.x - 40, y: point.y))
    try await eventually { (try? displayedText()) == nil }
  }
}

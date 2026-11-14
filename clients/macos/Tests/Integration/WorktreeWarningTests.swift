import AppKit
import ApplicationServices
import SwiftUI
import Vision
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
    let actual = try await host.hover(at: point, observe: displayedText)
    for expected in [
      "Changes unavailable", "Dirty failed", "PR unavailable", "PR failed", "Worktree failed",
    ] {
      XCTAssertTrue(actual.contains(expected), actual)
    }
    XCTAssertFalse(actual.contains("Old failure"))
    try await host.press("worktree:feature")
    XCTAssertEqual(model.selectedWorktree, "/repo/feature")
    try host.movePointer(to: CGPoint(x: point.x - 40, y: point.y))
    try await eventually { (try? displayedText()) == nil }
  }
}

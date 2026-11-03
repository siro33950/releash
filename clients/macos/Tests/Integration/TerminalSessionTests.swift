import AppKit
import Connect
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class TerminalSessionTests: XCTestCase {
  @MainActor func testFileDropSendsUnquotedPathsOnlyToTheDestinationOwner() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 720, height: 480), styleMask: .borderless,
      backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    defer { window.close() }
    let urls = [URL(fileURLWithPath: "/tmp/日本語 file.txt"), URL(fileURLWithPath: "/tmp/a'b;$x")]
    let drag = TerminalFileDrag()
    defer { drag.draggingPasteboard.releaseGlobally() }
    XCTAssertTrue(drag.draggingPasteboard.writeObjects(urls as [NSURL]))
    for agent in [false, true] {
      let session = model.terminal(path: "/repo", id: agent ? "agent" : "shell", agent: agent)
      window.contentView?.addSubview(session.screen)
      XCTAssertTrue(session.screen.registeredDraggedTypes.contains(.fileURL))
      try await eventually { session.hasForegroundProcess != nil }
      XCTAssertEqual(session.screen.draggingEntered(drag), .copy)
      XCTAssertTrue(session.screen.prepareForDragOperation(drag))
      XCTAssertTrue(session.screen.performDragOperation(drag))
      try await eventually {
        http.requests("WritePathsToTerminalSurface").count == (agent ? 2 : 1)
      }
      let request = try Releash_Client_V1_WritePathsToTerminalSurfaceRequest(
        serializedBytes: XCTUnwrap(http.requests("WritePathsToTerminalSurface").last?.message))
      XCTAssertEqual(request.owner, session.owner)
      XCTAssertEqual(request.paths.items, urls.map(\.path))
      session.screen.removeFromSuperview()
      session.stop()
    }
    XCTAssertTrue(http.requests("WriteTerminalSurface").isEmpty)
    await model.stop()
  }

  @MainActor func testFileDropRejectsTextRemoteURLsAndUnsupportedOperations() {
    let screen = TerminalScreen(frame: .zero)
    let drag = TerminalFileDrag()
    defer { drag.draggingPasteboard.releaseGlobally() }
    var dropped: [[String]] = []
    screen.dropPaths = {
      dropped.append($0)
      return true
    }
    for contents in [
      [], [NSString(string: "/tmp/plain text")], [NSURL(string: "https://example.com")!],
    ]
      as [[any NSPasteboardWriting]]
    {
      drag.draggingPasteboard.clearContents()
      XCTAssertTrue(drag.draggingPasteboard.writeObjects(contents))
      XCTAssertEqual(screen.draggingEntered(drag), [])
      XCTAssertFalse(screen.prepareForDragOperation(drag))
      XCTAssertFalse(screen.performDragOperation(drag))
    }
    drag.draggingPasteboard.clearContents()
    XCTAssertTrue(drag.draggingPasteboard.writeObjects([NSURL(fileURLWithPath: "/tmp/file")]))
    drag.draggingSourceOperationMask = .move
    XCTAssertEqual(screen.draggingEntered(drag), [])
    XCTAssertFalse(screen.performDragOperation(drag))
    XCTAssertTrue(dropped.isEmpty)
    drag.draggingSourceOperationMask = .copy
    screen.dropPaths = { _ in false }
    XCTAssertFalse(screen.performDragOperation(drag))
    screen.dropPaths = nil
    XCTAssertEqual(screen.draggingEntered(drag), [])
    XCTAssertFalse(screen.performDragOperation(drag))
  }

  @MainActor func testSnapshotRestoresSizeAndScreenAndReconnectResetsInputSequence() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let first = model.terminal(path: "/repo", id: "first")
    let second = model.terminal(path: "/repo", id: "second")
    try await eventually { first.hasForegroundProcess != nil && second.hasForegroundProcess != nil }
    var event = Releash_Client_V1_TerminalEvent()
    event.snapshot.cols = 100
    event.snapshot.rows = 30
    event.snapshot.processedReportUnits = 5000
    event.snapshot.replay = "saved output"
    first.receive(event)
    XCTAssertEqual(first.screen.terminalDimensions.cols, 100)
    XCTAssertTrue((first.screen.accessibilityValue() as? String)?.contains("saved output") == true)
    XCTAssertFalse(
      (second.screen.accessibilityValue() as? String)?.contains("saved output") == true)
    first.input("before\r")
    try await eventually { http.requests("WriteTerminalSurface").count == 1 }
    first.receive(event)
    first.input("same attachment\r")
    try await eventually { http.requests("WriteTerminalSurface").count == 2 }
    let resynchronized = try Releash_Client_V1_WriteTerminalSurfaceRequest(
      serializedBytes: http.requests("WriteTerminalSurface").last?.message ?? Data())
    XCTAssertEqual(resynchronized.sequence, 1)
    first.disconnect()
    first.input("after\r")
    first.receive(event)
    try await eventually { http.requests("WriteTerminalSurface").count == 3 }
    let last = try Releash_Client_V1_WriteTerminalSurfaceRequest(
      serializedBytes: http.requests("WriteTerminalSurface").last?.message ?? Data())
    XCTAssertEqual(last.sequence, 0)
    XCTAssertEqual(last.data, "after\r")
    first.clear()
    XCTAssertFalse((first.screen.accessibilityValue() as? String)?.contains("saved output") == true)
    event.exit.exitCode = 7
    first.receive(event)
    XCTAssertTrue(first.exited)
    XCTAssertTrue((first.screen.accessibilityValue() as? String)?.contains("code: 7") == true)
    first.stop()
    second.stop()
    await model.stop()
  }

  @MainActor func testAccessibilityPreservesWideAndComposedCharacters() {
    let screen = TerminalScreen(frame: .zero)
    screen.resize(cols: 80, rows: 24)
    screen.feed(text: "漢字 👩‍💻 e\u{301}")
    let value = screen.accessibilityValue() as? String ?? ""
    XCTAssertTrue(value.contains("漢字"))
    XCTAssertTrue(value.contains("👩‍💻"))
    XCTAssertTrue(value.contains("e\u{301}"))
  }

  @MainActor func testWorkspaceLoadingDoesNotStartShellForPreviousTabs() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    try model.panes.restore(
      .pane(
        id: "old-pane", tabs: [PaneTab(id: "old-terminal", kind: .terminal)], active: "old-terminal"
      ))
    model.selectedWorktree = "/repo/new"
    model.layoutLoaded = false
    let host = NSHostingView(rootView: WorkspacePaneView(model: model))
    host.frame = NSRect(x: 0, y: 0, width: 720, height: 480)
    host.layoutSubtreeIfNeeded()
    await Task.yield()
    XCTAssertTrue(http.requests("GetOrSpawnTerminalSurface").isEmpty)
    model.layoutLoaded = true
    host.layoutSubtreeIfNeeded()
    try await eventually { http.requests("GetOrSpawnTerminalSurface").count == 1 }
    model.terminal(path: "/repo/new", id: "old-terminal").stop()
    await model.stop()
  }

  @MainActor func testOnlyFocusedTerminalTakesKeyboardOnAttach() {
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 720, height: 480), styleMask: .borderless,
      backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    defer { window.close() }
    let active = TerminalScreen(frame: NSRect(x: 0, y: 0, width: 360, height: 480))
    let other = TerminalScreen(frame: NSRect(x: 360, y: 0, width: 360, height: 480))
    active.setFocused(true)
    window.contentView?.addSubview(active)
    window.contentView?.addSubview(other)
    XCTAssertTrue(window.firstResponder === active)
    active.setFocused(false)
    other.setFocused(true)
    XCTAssertTrue(window.firstResponder === other)
  }

  @MainActor func testStreamRecoveryUsesNewAttachmentAndIgnoresOldEvents() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let session = model.terminal(path: "/repo", id: "recover")
    session.screen.resize(cols: 80, rows: 24)
    try await eventually { session.hasForegroundProcess != nil }
    let previous = try XCTUnwrap(model.terminalAttachment(path: "/repo", id: "recover"))
    session.input("before\r")
    try await eventually { http.requests("WriteTerminalSurface").count == 1 }
    let stream = http.state.withLock { storage in
      storage.terminalReplay = "output while disconnected"
      return storage.streams.last!
    }
    stream.receiveClose(
      .unavailable, [:], ConnectError(code: .unavailable, message: "disconnected"))
    try await eventually { model.reconnecting }
    session.input("after\r")
    try await eventually {
      model.terminalAttachment(path: "/repo", id: "recover") != previous
        && session.hasForegroundProcess != nil && !model.reconnecting
        && http.requests("WriteTerminalSurface").count == 2
    }
    let current = try XCTUnwrap(model.terminalAttachment(path: "/repo", id: "recover"))
    XCTAssertTrue(
      (session.screen.accessibilityValue() as? String)?.contains("output while disconnected")
        == true)
    let write = try Releash_Client_V1_WriteTerminalSurfaceRequest(
      serializedBytes: http.requests("WriteTerminalSurface").last?.message ?? Data())
    XCTAssertEqual(write.attachmentID, current)
    XCTAssertEqual(write.sequence, 0)
    XCTAssertEqual(write.data, "after\r")
    var stale = Releash_Client_V1_StateSubscriptionEvent()
    stale.subscriptionID = previous
    stale.change.payload.terminal.output.data = "stale output"
    try TestHTTPClient.send(stale, on: http.state.withLock { $0.streams.last! })
    stale.change.payload.terminal.status.cwd = "/stale"
    stale.change.payload.terminal.status.hasForegroundProcess_p = true
    try TestHTTPClient.send(stale, on: http.state.withLock { $0.streams.last! })
    var fresh = Releash_Client_V1_StateSubscriptionEvent()
    fresh.subscriptionID = current
    fresh.change.payload.terminal.output.data = String(repeating: "x", count: 5000)
    try TestHTTPClient.send(fresh, on: http.state.withLock { $0.streams.last! })
    try await eventually { http.requests("ReportTerminalProcessed").count == 1 }
    XCTAssertFalse(
      (session.screen.accessibilityValue() as? String)?.contains("stale output") == true)
    XCTAssertEqual(session.cwd, "/repo")
    XCTAssertEqual(session.hasForegroundProcess, false)
    let report = try Releash_Client_V1_ReportTerminalProcessedRequest(
      serializedBytes: http.requests("ReportTerminalProcessed").last?.message ?? Data())
    XCTAssertEqual(report.subscriptionID, current)
    XCTAssertEqual(report.units, 5000)
    XCTAssertEqual(
      http.state.withLock { $0.subscriptions.filter { $0.target == "workspace-terminal" }.count }, 2
    )
    session.stop()
    await model.stop()
  }

  @MainActor func testServerResizeAppliesScreenSize() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let session = model.terminal(path: "/repo", id: "size")
    try await eventually { session.hasForegroundProcess != nil }
    var event = Releash_Client_V1_TerminalEvent()
    event.resize.cols = 90
    event.resize.rows = 25
    session.receive(event)
    XCTAssertEqual(session.screen.terminalDimensions.cols, 90)
    XCTAssertEqual(session.screen.terminalDimensions.rows, 25)
    session.stop()
    await model.stop()
  }

  @MainActor func testHiddenTerminalRelinquishesKeyboardWithoutTakingOtherControlsFocus() {
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 400, height: 300), styleMask: .titled,
      backing: .buffered, defer: false)
    let screen = TerminalScreen(frame: window.contentView!.bounds)
    window.contentView!.addSubview(screen)
    screen.setFocused(true)
    XCTAssertTrue(window.firstResponder === screen)
    screen.setFocused(false)
    XCTAssertFalse(window.firstResponder === screen)
    screen.setFocused(true)
    XCTAssertTrue(window.firstResponder === screen)
    let other = NSTextView(frame: window.contentView!.bounds)
    window.contentView!.addSubview(other)
    window.makeFirstResponder(other)
    screen.setFocused(false)
    XCTAssertTrue(window.firstResponder === other)
  }
}

@MainActor private final class TerminalFileDrag: NSObject, NSDraggingInfo {
  let draggingPasteboard = NSPasteboard.withUniqueName()
  var draggingSourceOperationMask: NSDragOperation = .copy
  var draggingDestinationWindow: NSWindow? { nil }
  var draggingLocation: NSPoint { .zero }
  var draggedImageLocation: NSPoint { .zero }
  nonisolated var draggedImage: NSImage? { nil }
  var draggingSource: Any? { nil }
  var draggingSequenceNumber: Int { 0 }
  var draggingFormation: NSDraggingFormation = .none
  var animatesToDestination = false
  var numberOfValidItemsForDrop = 0
  var springLoadingHighlight: NSSpringLoadingHighlight { .none }
  func slideDraggedImage(to screenPoint: NSPoint) {}
  nonisolated override func namesOfPromisedFilesDropped(atDestination dropDestination: URL)
    -> [String]?
  { nil }
  func resetSpringLoading() {}
  func enumerateDraggingItems(
    options enumOpts: NSDraggingItemEnumerationOptions, for view: NSView?,
    classes classArray: [AnyClass],
    searchOptions: [NSPasteboard.ReadingOptionKey: Any],
    using block: (NSDraggingItem, Int, UnsafeMutablePointer<ObjCBool>) -> Void
  ) {}
}

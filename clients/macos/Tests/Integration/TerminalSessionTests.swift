import AppKit
import Connect
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class TerminalSessionTests: XCTestCase {
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
    XCTAssertEqual(first.screen.getTerminal().cols, 100)
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

  @MainActor func testServerResizePreservesOutputAttributes() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let session = model.terminal(path: "/repo", id: "color")
    try await eventually { session.hasForegroundProcess != nil }
    var event = Releash_Client_V1_TerminalEvent()
    event.output.data = "\u{1b}[H\u{1b}[31mA"
    session.receive(event)
    let terminal = session.screen.getTerminal()
    let before = try XCTUnwrap(terminal.getLine(row: 0))[0].attribute
    event.resize.cols = 90
    event.resize.rows = 25
    session.receive(event)
    event.output.data = "B"
    session.receive(event)
    let line = try XCTUnwrap(terminal.getLine(row: 0))
    XCTAssertEqual(terminal.getCharacter(for: line[1]), "B")
    XCTAssertEqual(line[1].attribute, before)
    XCTAssertEqual(terminal.cols, 90)
    XCTAssertEqual(terminal.rows, 25)
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

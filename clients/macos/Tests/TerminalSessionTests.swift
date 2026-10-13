import AppKit
import Connect
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class TerminalSessionTests: XCTestCase {
  @MainActor func testStartupInputIsOrderedAndOutputReportsUTF16Units() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let session = model.terminal(path: "/repo", id: "first")
    session.input("echo ")
    session.input("ready\r")
    try await eventually { !http.requests("WriteTerminalSurface").isEmpty }
    let writes = try http.requests("WriteTerminalSurface").map {
      try Releash_Client_V1_WriteTerminalSurfaceRequest(serializedBytes: $0.message ?? Data())
    }
    XCTAssertEqual(writes.map(\.data).joined(), "echo ready\r")
    XCTAssertEqual(writes.map(\.sequence), Array(0..<UInt64(writes.count)))
    XCTAssertEqual(writes.first?.owner.workspaceTerminal.terminalID, "first")
    let attachment = try XCTUnwrap(model.terminalAttachment(path: "/repo", id: "first"))
    var event = Releash_Client_V1_TerminalEvent()
    event.output.data = String(repeating: "😀", count: 7501)
    session.receive(event)
    try await eventually { http.requests("ReportTerminalProcessed").count == 3 }
    for request in http.requests("ReportTerminalProcessed") {
      let report = try Releash_Client_V1_ReportTerminalProcessedRequest(serializedBytes: request.message ?? Data())
      XCTAssertEqual(report.units, 5000)
      XCTAssertEqual(report.subscriptionID, attachment)
    }
    session.stop()
    await model.stop()
    XCTAssertTrue(http.requests("KillTerminalSurface").isEmpty)
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
    XCTAssertEqual(first.screen.getTerminal().cols, 100)
    XCTAssertTrue((first.screen.accessibilityValue() as? String)?.contains("saved output") == true)
    XCTAssertFalse((second.screen.accessibilityValue() as? String)?.contains("saved output") == true)
    first.input("before\r")
    try await eventually { http.requests("WriteTerminalSurface").count == 1 }
    first.receive(event)
    first.input("same attachment\r")
    try await eventually { http.requests("WriteTerminalSurface").count == 2 }
    let resynchronized = try Releash_Client_V1_WriteTerminalSurfaceRequest(serializedBytes: http.requests("WriteTerminalSurface").last?.message ?? Data())
    XCTAssertEqual(resynchronized.sequence, 1)
    first.disconnect()
    first.input("after\r")
    first.receive(event)
    try await eventually { http.requests("WriteTerminalSurface").count == 3 }
    let last = try Releash_Client_V1_WriteTerminalSurfaceRequest(serializedBytes: http.requests("WriteTerminalSurface").last?.message ?? Data())
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
}


extension TerminalSessionTests {
  @MainActor func testAccessibilityPreservesWideAndComposedCharacters() {
    let screen = TerminalScreen(frame: .zero)
    screen.resize(cols: 80, rows: 24)
    screen.feed(text: "漢字 👩‍💻 e\u{301}")
    let value = screen.accessibilityValue() as? String ?? ""
    XCTAssertTrue(value.contains("漢字"))
    XCTAssertTrue(value.contains("👩‍💻"))
    XCTAssertTrue(value.contains("e\u{301}"))
  }
}

extension TerminalSessionTests {
  @MainActor func testIdleCloseStopsOnlyItsOwnerAndPreservesTabOnFailure() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectedWorktree = "/repo"
    let session = model.terminal(path: "/repo", id: "first")
    try await eventually { session.hasForegroundProcess == false }
    model.confirmTerminalClose = { XCTFail("Idle shell must not require confirmation"); return false }
    let tab = PaneTab(id: "first", kind: .terminal)
    http.state.withLock { $0.failedMethod = "KillTerminalSurface" }
    let failed = await model.closeTerminal(tab)
    XCTAssertFalse(failed)
    XCTAssertTrue(model.failure?.contains("KillTerminalSurface failed") == true)
    http.state.withLock { $0.failedMethod = nil }
    let closed = await model.closeTerminal(tab)
    XCTAssertTrue(closed)
    for request in http.requests("KillTerminalSurface") {
      let kill = try Releash_Client_V1_KillTerminalSurfaceRequest(serializedBytes: request.message ?? Data())
      XCTAssertEqual(kill.owner.workspaceTerminal.workspacePath, "/repo")
      XCTAssertEqual(kill.owner.workspaceTerminal.terminalID, "first")
    }
    await model.stop()
  }
}


extension TerminalSessionTests {
  @MainActor func testWorkspaceLoadingDoesNotStartShellForPreviousTabs() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    try model.panes.restore(.pane(id: "old-pane", tabs: [PaneTab(id: "old-terminal", kind: .terminal)], active: "old-terminal"))
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
}


extension TerminalSessionTests {
  @MainActor func testOnlyFocusedTerminalTakesKeyboardOnAttach() {
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 720, height: 480), styleMask: .borderless, backing: .buffered, defer: false)
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
}

extension TerminalSessionTests {
  @MainActor func testStatusAndDisplaySizeReachTheSubscribedTab() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let session = model.terminal(path: "/repo", id: "sized")
    session.sizeChanged(source: session.screen, newCols: 110, newRows: 32)
    try await eventually { session.hasForegroundProcess != nil && !http.requests("ResizeTerminalSurface").isEmpty }
    let firstResize = try Releash_Client_V1_ResizeTerminalSurfaceRequest(serializedBytes: http.requests("ResizeTerminalSurface").first?.message ?? Data())
    XCTAssertEqual(firstResize.cols, 110)
    XCTAssertEqual(firstResize.rows, 32)
    var status = Releash_Client_V1_TerminalEvent()
    status.status.cwd = "/repo/moved"
    status.status.hasForegroundProcess_p = true
    session.receive(status)
    XCTAssertEqual(session.cwd, "/repo/moved")
    XCTAssertEqual(session.hasForegroundProcess, true)
    session.sizeChanged(source: session.screen, newCols: 120, newRows: 40)
    try await eventually {
      guard let data = http.requests("ResizeTerminalSurface").last?.message,
        let resize = try? Releash_Client_V1_ResizeTerminalSurfaceRequest(serializedBytes: data) else { return false }
      return resize.cols == 120 && resize.rows == 40
    }
    var empty = Releash_Client_V1_TerminalEvent()
    empty.snapshot.processedReportUnits = 5000
    session.receive(empty)
    let count = http.requests("ResizeTerminalSurface").count
    try await eventually { http.requests("ResizeTerminalSurface").count == count + 1 }
    for request in http.requests("ResizeTerminalSurface") {
      let resize = try Releash_Client_V1_ResizeTerminalSurfaceRequest(serializedBytes: request.message ?? Data())
      XCTAssertEqual(resize.owner.workspaceTerminal.workspacePath, "/repo")
      XCTAssertEqual(resize.owner.workspaceTerminal.terminalID, "sized")
    }
    let initial = try Releash_Client_V1_ResizeTerminalSurfaceRequest(serializedBytes: http.requests("ResizeTerminalSurface").last?.message ?? Data())
    XCTAssertEqual(initial.cols, 120)
    XCTAssertEqual(initial.rows, 40)
    var received = Releash_Client_V1_TerminalEvent()
    received.resize.cols = 90
    received.resize.rows = 25
    session.receive(received)
    await Task.yield()
    XCTAssertEqual(http.requests("ResizeTerminalSurface").count, count + 1)
    session.stop()
    await model.stop()
  }

  @MainActor func testBusyCloseConfirmsAndCancellationPreservesTabAndAttachment() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectedWorktree = "/repo"
    try model.panes.restore(.empty())
    let pane = try XCTUnwrap(model.panes.controller.allPaneIds.first)
    model.panes.open(.terminal, in: pane)
    let tab = try XCTUnwrap(try model.panes.layout().tabs.first)
    let session = model.terminal(path: "/repo", id: tab.id)
    try await eventually { session.hasForegroundProcess != nil }
    var event = Releash_Client_V1_TerminalEvent()
    event.status.hasForegroundProcess_p = true
    session.receive(event)
    let attachment = model.terminalAttachment(path: "/repo", id: tab.id)
    var confirmations = 0
    var approved = false
    model.confirmTerminalClose = { confirmations += 1; return approved }
    model.panes.requestClose = { await model.closeTerminal($0) }
    model.panes.closeCurrentTab()
    try await eventually { confirmations == 1 }
    await Task.yield()
    XCTAssertTrue(http.requests("KillTerminalSurface").isEmpty)
    XCTAssertEqual(try model.panes.layout().tabs.map(\.id), [tab.id])
    XCTAssertEqual(model.terminalAttachment(path: "/repo", id: tab.id), attachment)
    approved = true
    model.panes.closeCurrentTab()
    try await eventually { model.panes.controller.allTabIds.isEmpty }
    XCTAssertEqual(confirmations, 2)
    let kill = try Releash_Client_V1_KillTerminalSurfaceRequest(serializedBytes: http.requests("KillTerminalSurface").last?.message ?? Data())
    XCTAssertEqual(kill.owner, session.owner)
    await model.stop()
  }

  @MainActor func testLinkDelegateRequestsTheDisplayedURLWithoutOpeningAnApp() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    var opened: [URL] = []
    let session = TerminalSession(model: model, path: "/repo", id: "link", openLink: { opened.append($0) })
    session.requestOpenLink(source: session.screen, link: "https://example.com/path?q=terminal", params: [:])
    XCTAssertEqual(opened, [URL(string: "https://example.com/path?q=terminal")!])
    session.stop()
    await model.stop()
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
    stream.receiveClose(.unavailable, [:], ConnectError(code: .unavailable, message: "disconnected"))
    try await eventually { model.reconnecting }
    session.input("after\r")
    try await eventually { model.terminalAttachment(path: "/repo", id: "recover") != previous && session.hasForegroundProcess != nil && !model.reconnecting && http.requests("WriteTerminalSurface").count == 2 }
    let current = try XCTUnwrap(model.terminalAttachment(path: "/repo", id: "recover"))
    XCTAssertTrue((session.screen.accessibilityValue() as? String)?.contains("output while disconnected") == true)
    let write = try Releash_Client_V1_WriteTerminalSurfaceRequest(serializedBytes: http.requests("WriteTerminalSurface").last?.message ?? Data())
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
    XCTAssertFalse((session.screen.accessibilityValue() as? String)?.contains("stale output") == true)
    XCTAssertEqual(session.cwd, "/repo")
    XCTAssertEqual(session.hasForegroundProcess, false)
    let report = try Releash_Client_V1_ReportTerminalProcessedRequest(serializedBytes: http.requests("ReportTerminalProcessed").last?.message ?? Data())
    XCTAssertEqual(report.subscriptionID, current)
    XCTAssertEqual(report.units, 5000)
    XCTAssertEqual(http.state.withLock { $0.subscriptions.filter { $0.target == "workspace-terminal" }.count }, 2)
    session.stop()
    await model.stop()
  }
}

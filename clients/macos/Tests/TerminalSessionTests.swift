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
  @MainActor func testExitUsesTheServerStatus() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let session = model.terminal(path: "/repo", id: "exit-status")
    try await eventually { session.hasForegroundProcess != nil }
    var event = Releash_Client_V1_TerminalEvent()
    event.exit.exitCode = 0
    event.exit.status.cwd = "/server"
    event.exit.status.hasForegroundProcess_p = true
    session.receive(event)
    XCTAssertEqual(session.hasForegroundProcess, true)
    XCTAssertEqual(session.cwd, "/server")
    event.exit.status.hasForegroundProcess_p = false
    session.receive(event)
    XCTAssertEqual(session.hasForegroundProcess, false)
    event.exit.clearStatus()
    session.receive(event)
    XCTAssertNil(session.hasForegroundProcess)
    XCTAssertNil(session.cwd)
    session.stop()
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
    let allowed = ["http://example.com", "https://example.com/path?q=terminal", "mailto:dev@example.com", "tel:+123456789", "HTTPS://example.com"]
    for link in allowed { session.requestOpenLink(source: session.screen, link: link, params: [:]) }
    XCTAssertEqual(opened, allowed.map { URL(string: $0)! })
    for link in ["file:///System/Applications/Calculator.app", "releash-custom://run", "/relative"] {
      session.requestOpenLink(source: session.screen, link: link, params: [:])
    }
    XCTAssertEqual(opened.count, allowed.count)
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

extension TerminalSessionTests {
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

  @MainActor private func closePaneWithChangingTabs(move: Bool, cancel: Bool = false) async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectedWorktree = "/repo"
    let paneID = UUID().uuidString
    let otherID = UUID().uuidString
    let first = PaneTab(id: UUID().uuidString, kind: .terminal)
    let extra = PaneTab(id: UUID().uuidString, kind: .terminal)
    let file = PaneTab(id: UUID().uuidString, kind: .file)
    func layout(moved: Bool) -> PaneLayout {
      .split(id: "split", axis: .horizontal, ratio: 0.5,
        first: .pane(id: paneID, tabs: moved ? [first, extra] : [first], active: first.id),
        second: .pane(id: otherID, tabs: move && !moved ? [file, extra] : [file], active: file.id))
    }
    try model.panes.restore(layout(moved: false))
    let pane = try XCTUnwrap(model.panes.controller.allPaneIds.first { $0.id.uuidString == paneID })
    let firstSession = model.terminal(path: "/repo", id: first.id)
    try await eventually { firstSession.hasForegroundProcess != nil }
    var release: CheckedContinuation<Void, Never>?
    var cancelled = false
    model.panes.requestClose = { tab in
      if tab.id == first.id { await withCheckedContinuation { release = $0 } }
      let result = await model.closeTerminal(tab)
      if !result { cancelled = true }
      return result
    }
    model.panes.controller.closePane(pane)
    try await eventually { release != nil }
    let added: PaneTab
    if move {
      try model.panes.restore(layout(moved: true))
      added = extra
    } else {
      model.panes.open(.terminal, in: pane)
      added = try XCTUnwrap(model.panes.controller.tabs(inPane: pane).compactMap { model.panes.tab($0.id) }.first { $0.id != first.id })
    }
    let addedSession = model.terminal(path: "/repo", id: added.id)
    try await eventually { addedSession.hasForegroundProcess != nil }
    if cancel {
      var event = Releash_Client_V1_TerminalEvent()
      event.status.hasForegroundProcess_p = true
      addedSession.receive(event)
      model.confirmTerminalClose = { false }
    }
    release?.resume()
    try await eventually { !http.requests("KillTerminalSurface").isEmpty }
    if cancel {
      try await eventually { cancelled }
      XCTAssertTrue(model.panes.controller.allPaneIds.contains(pane))
      XCTAssertEqual(Set(model.panes.controller.tabs(inPane: pane).compactMap { model.panes.tab($0.id)?.id }), Set([added.id]))
      XCTAssertEqual(http.requests("KillTerminalSurface").count, 1)
      model.confirmTerminalClose = { true }
      model.panes.controller.closePane(pane)
      try await eventually { !model.panes.controller.allPaneIds.contains(pane) }
      let kills = try http.requests("KillTerminalSurface").map {
        try Releash_Client_V1_KillTerminalSurfaceRequest(serializedBytes: $0.message ?? Data()).owner.workspaceTerminal.terminalID
      }
      XCTAssertEqual(kills, [first.id, added.id])
    } else {
      try await eventually { !model.panes.controller.allPaneIds.contains(pane) }
      let kills = try http.requests("KillTerminalSurface").map {
        try Releash_Client_V1_KillTerminalSurfaceRequest(serializedBytes: $0.message ?? Data()).owner.workspaceTerminal.terminalID
      }
      XCTAssertEqual(kills, [first.id, added.id])
    }
    await model.stop()
  }

  @MainActor func testPaneCloseStopsTerminalAddedWhileWaiting() async throws {
    try await closePaneWithChangingTabs(move: false)
  }
  @MainActor func testPaneCloseStopsTerminalMovedWhileWaiting() async throws {
    try await closePaneWithChangingTabs(move: true)
  }
  @MainActor func testPaneCloseCancellationRemovesStoppedTabAndCanCloseRemainingPane() async throws {
    try await closePaneWithChangingTabs(move: false, cancel: true)
  }
}


extension TerminalSessionTests {
  @MainActor func testExitedSnapshotAndExitEventCloseWithoutConfirmationAfterReconnect() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectedWorktree = "/repo"
    model.confirmTerminalClose = { XCTFail("Exited terminal must not require confirmation"); return false }
    for restored in [false, true] {
      let id = restored ? "snapshot" : "exit"
      let session = model.terminal(path: "/repo", id: id)
      try await eventually { session.hasForegroundProcess != nil }
      var event = Releash_Client_V1_TerminalEvent()
      event.status.hasForegroundProcess_p = true
      session.receive(event)
      XCTAssertEqual(session.hasForegroundProcess, true)
      if restored {
        session.disconnect()
        event.snapshot.isExited = true
        event.snapshot.exitCode = 0
        event.snapshot.processedReportUnits = 5000
        event.snapshot.status.cwd = "/last"
        event.snapshot.status.hasForegroundProcess_p = false
      } else {
        event.exit.exitCode = 0
        event.exit.status.cwd = "/last"
        event.exit.status.hasForegroundProcess_p = false
      }
      session.receive(event)
      XCTAssertTrue(session.exited)
      XCTAssertEqual(session.hasForegroundProcess, false)
      let closed = await model.closeTerminal(PaneTab(id: id, kind: .terminal))
      XCTAssertTrue(closed)
    }
    let owners = try http.requests("KillTerminalSurface").map {
      try Releash_Client_V1_KillTerminalSurfaceRequest(serializedBytes: $0.message ?? Data()).owner.workspaceTerminal.terminalID
    }
    XCTAssertEqual(owners, ["exit", "snapshot"])
    await model.stop()
  }
}

extension TerminalSessionTests {
  @MainActor func testAgentTerminalUsesSessionOwnerAndPreservesInputProtocol() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let terminal = model.terminal(path: "/repo", id: "agent-session", agent: true)
    terminal.input("hello\r")
    try await eventually { !http.requests("WriteTerminalSurface").isEmpty }
    let write = try Releash_Client_V1_WriteTerminalSurfaceRequest(serializedBytes: XCTUnwrap(http.requests("WriteTerminalSurface").last?.message))
    XCTAssertEqual(write.owner.session.sessionID, "agent-session")
    XCTAssertEqual(write.owner.session.workspacePath, "/repo")
    XCTAssertEqual(write.sequence, 0)
    XCTAssertEqual(write.data, "hello\r")
    model.releaseAgentTerminal(path: "/repo", id: "agent-session")
    XCTAssertTrue(http.requests("KillTerminalSurface").isEmpty)
    await model.stop()
  }
}

extension TerminalSessionTests {
  @MainActor func testHiddenTerminalRelinquishesKeyboardWithoutTakingOtherControlsFocus() {
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 300), styleMask: .titled, backing: .buffered, defer: false)
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

extension TerminalSessionTests {
  @MainActor func testDeletedAgentSessionDoesNotBlockOtherSubscriptionsDuringRecovery() async throws {
    // Given
    let http = TestHTTPClient()
    var session = Releash_Client_V1_AgentSessionItemDto(); session.id = "deleted"
    http.state.withLock { $0.sessions["deleted"] = session }
    let model = try await testModel(http)
    var receivedSession = false
    var receivedDeletion = false
    let subscription = model.subscribe("agent-session", ["deleted"]) { payload in
      if case .agentSession(let value) = payload.value {
        receivedSession = value.hasValue
        receivedDeletion = !value.hasValue
      }
    }
    let terminal = model.terminal(path: "/repo", id: "deleted", agent: true)
    try await eventually { receivedSession && terminal.hasForegroundProcess != nil }
    let previous = try XCTUnwrap(model.terminalAttachment(path: "/repo", id: "deleted", agent: true))
    let stream = http.state.withLock { state in
      state.sessions.removeValue(forKey: "deleted")
      state.failedMethod = "OpenAgentSession"
      var repo = Releash_Client_V1_WorkspaceRepositoryList(); repo.path = "/recovered"
      state.repositories = [repo]
      return state.streams.last!
    }
    // When
    stream.receiveClose(.unavailable, [:], ConnectError(code: .unavailable, message: "disconnected"))
    // Then
    try await eventually {
      receivedDeletion && model.repositories.first?.path == "/recovered" && !model.reconnecting
        && model.terminalAttachment(path: "/repo", id: "deleted", agent: true) != previous
    }
    model.unsubscribe(subscription)
    model.releaseAgentTerminal(path: "/repo", id: "deleted")
    await model.stop()
  }
}

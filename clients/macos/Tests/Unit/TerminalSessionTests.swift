import AppKit
import Connect
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class TerminalSessionTests: XCTestCase {
  @MainActor func testDroppedPathsUseEachOwnerAndPreserveInputOrderAndSequence() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let paths = ["/tmp/日本語 file.txt", "/tmp/a'b;$file"]
    for agent in [false, true] {
      let session = model.terminal(path: "/repo", id: agent ? "agent" : "shell", agent: agent)
      XCTAssertFalse(session.writePaths(paths))
      try await eventually { session.hasForegroundProcess != nil }
      session.input("before ")
      XCTAssertTrue(session.writePaths(paths))
      session.input(" after")
      try await eventually {
        http.requests("WriteTerminalSurface").count == (agent ? 4 : 2)
      }
      let drop = try Releash_Client_V1_WritePathsToTerminalSurfaceRequest(
        serializedBytes: XCTUnwrap(http.requests("WritePathsToTerminalSurface").last?.message))
      XCTAssertEqual(drop.owner, session.owner)
      XCTAssertEqual(drop.paths.items, paths)
      let writes = try http.requests("WriteTerminalSurface").suffix(2).map {
        try Releash_Client_V1_WriteTerminalSurfaceRequest(serializedBytes: $0.message ?? Data())
      }
      XCTAssertEqual(writes.map(\.data), ["before ", " after"])
      XCTAssertEqual(writes.map(\.sequence), [0, 1])
      let methods = http.state.withLock {
        $0.requests.map(\.url.lastPathComponent).filter {
          ["WriteTerminalSurface", "WritePathsToTerminalSurface"].contains($0)
        }
      }
      XCTAssertEqual(
        Array(methods.suffix(3)),
        ["WriteTerminalSurface", "WritePathsToTerminalSurface", "WriteTerminalSurface"])
      XCTAssertFalse(session.writePaths([]))
      XCTAssertTrue(session.writePaths(["/tmp/stale"]))
      session.disconnect()
      try await eventually { session.dropFailure?.contains("/tmp/stale") == true }
      XCTAssertFalse(session.writePaths(paths))
      var event = Releash_Client_V1_TerminalEvent()
      event.snapshot.isExited = true
      session.receive(event)
      XCTAssertTrue(session.dropFailure?.contains("/tmp/stale") == true)
      XCTAssertFalse(session.writePaths(paths))
      session.stop()
    }
    await model.stop()
    XCTAssertEqual(http.requests("WritePathsToTerminalSurface").count, 2)
  }

  @MainActor func testDroppedPathFailureRestartsAttachmentAndReportsError() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let session = model.terminal(path: "/repo", id: "drop-failure")
    try await eventually { session.hasForegroundProcess != nil }
    let previous = model.terminalAttachment(path: "/repo", id: "drop-failure")
    http.state.withLock {
      $0.failedMethod = "WritePathsToTerminalSurface"
      $0.failedSubscriptionTargets["workspace-terminal"] = 1
    }
    XCTAssertTrue(session.writePaths(["/tmp/file"]))
    try await eventually {
      model.terminalAttachment(path: "/repo", id: "drop-failure") != previous
    }
    XCTAssertEqual(http.requests("WritePathsToTerminalSurface").count, 1)
    XCTAssertTrue(session.failure?.contains("WritePathsToTerminalSurface failed") == true)
    XCTAssertTrue(session.dropFailure?.contains("/tmp/file") == true)
    let stream = http.state.withLock { $0.streams.last! }
    stream.receiveClose(
      .unavailable, [:], ConnectError(code: .unavailable, message: "disconnected"))
    try await eventually { session.hasForegroundProcess != nil && session.failure == nil }
    XCTAssertTrue(session.dropFailure?.contains("確認できませんでした") == true)
    session.stop()
    await model.stop()
  }

  @MainActor func testQueuedDropAfterExitDoesNotSendOrRestartSubscription() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let session = model.terminal(path: "/repo", id: "exit-before-drop")
    try await eventually { session.hasForegroundProcess != nil }
    http.state.withLock { $0.deferredTerminalMethod = "WriteTerminalSurface" }
    session.input("before")
    try await eventually { http.state.withLock { $0.pendingTerminal != nil } }
    XCTAssertTrue(session.writePaths(["/tmp/queued"]))
    let attachment = model.terminalAttachment(path: "/repo", id: "exit-before-drop")
    var event = Releash_Client_V1_TerminalEvent()
    event.exit.exitCode = 0
    session.receive(event)
    let pending = try XCTUnwrap(http.state.withLock { $0.pendingTerminal })
    pending(
      HTTPResponse(
        code: .ok, headers: ["content-type": ["application/proto"]], message: Data(),
        trailers: [:], error: nil, tracingInfo: .init(httpStatus: 200)))
    try await eventually { session.dropFailure?.contains("端末が終了") == true }
    XCTAssertTrue(http.requests("WritePathsToTerminalSurface").isEmpty)
    XCTAssertEqual(model.terminalAttachment(path: "/repo", id: "exit-before-drop"), attachment)
    XCTAssertNil(session.failure)
    session.stop()
    await model.stop()
  }

  @MainActor func testInFlightDropFailureAfterReconnectRetainsUncertainResultWithoutResend()
    async throws
  {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let session = model.terminal(path: "/repo", id: "in-flight-drop")
    try await eventually { session.hasForegroundProcess != nil }
    http.state.withLock { $0.deferredTerminalMethod = "WritePathsToTerminalSurface" }
    XCTAssertTrue(session.writePaths(["/tmp/in-flight"]))
    try await eventually { http.state.withLock { $0.pendingTerminal != nil } }
    let previous = model.terminalAttachment(path: "/repo", id: "in-flight-drop")
    model.restartTerminalSubscription(path: "/repo", id: "in-flight-drop", agent: false)
    try await eventually {
      model.terminalAttachment(path: "/repo", id: "in-flight-drop") != previous
        && session.hasForegroundProcess != nil
    }
    let current = model.terminalAttachment(path: "/repo", id: "in-flight-drop")
    let pending = try XCTUnwrap(http.state.withLock { $0.pendingTerminal })
    pending(TestHTTPClient.failure("connection changed"))
    try await eventually { session.dropFailure?.contains("/tmp/in-flight") == true }
    XCTAssertTrue(session.dropFailure?.contains("確認できませんでした") == true)
    XCTAssertNil(session.failure)
    XCTAssertEqual(model.terminalAttachment(path: "/repo", id: "in-flight-drop"), current)
    XCTAssertEqual(http.requests("WritePathsToTerminalSurface").count, 1)
    session.stop()
    await model.stop()
  }

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
      let report = try Releash_Client_V1_ReportTerminalProcessedRequest(
        serializedBytes: request.message ?? Data())
      XCTAssertEqual(report.units, 5000)
      XCTAssertEqual(report.subscriptionID, attachment)
    }
    session.stop()
    await model.stop()
    XCTAssertTrue(http.requests("KillTerminalSurface").isEmpty)
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
  @MainActor func testIdleCloseStopsOnlyItsOwnerAndPreservesTabOnFailure() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectedWorktree = "/repo"
    let session = model.terminal(path: "/repo", id: "first")
    try await eventually { session.hasForegroundProcess == false }
    model.confirmTerminalClose = {
      XCTFail("Idle shell must not require confirmation")
      return false
    }
    let tab = PaneTab(id: "first", kind: .terminal)
    http.state.withLock { $0.failedMethod = "KillTerminalSurface" }
    let failed = await model.closeTerminal(tab)
    XCTAssertFalse(failed)
    XCTAssertTrue(model.failure?.contains("KillTerminalSurface failed") == true)
    http.state.withLock { $0.failedMethod = nil }
    let closed = await model.closeTerminal(tab)
    XCTAssertTrue(closed)
    for request in http.requests("KillTerminalSurface") {
      let kill = try Releash_Client_V1_KillTerminalSurfaceRequest(
        serializedBytes: request.message ?? Data())
      XCTAssertEqual(kill.owner.workspaceTerminal.workspacePath, "/repo")
      XCTAssertEqual(kill.owner.workspaceTerminal.terminalID, "first")
    }
    await model.stop()
  }
}

extension TerminalSessionTests {
  @MainActor func testStatusAndDisplaySizeReachTheSubscribedTab() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let session = model.terminal(path: "/repo", id: "sized")
    session.sizeChanged(source: session.screen, newCols: 110, newRows: 32)
    try await eventually {
      session.hasForegroundProcess != nil && !http.requests("ResizeTerminalSurface").isEmpty
    }
    let firstResize = try Releash_Client_V1_ResizeTerminalSurfaceRequest(
      serializedBytes: http.requests("ResizeTerminalSurface").first?.message ?? Data())
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
        let resize = try? Releash_Client_V1_ResizeTerminalSurfaceRequest(serializedBytes: data)
      else { return false }
      return resize.cols == 120 && resize.rows == 40
    }
    let count = http.requests("ResizeTerminalSurface").count
    var empty = Releash_Client_V1_TerminalEvent()
    empty.snapshot.processedReportUnits = 5000
    session.receive(empty)
    session.input("after resend")
    try await eventually { http.requests("WriteTerminalSurface").count == 1 }
    XCTAssertEqual(http.requests("ResizeTerminalSurface").count, count)
    for request in http.requests("ResizeTerminalSurface") {
      let resize = try Releash_Client_V1_ResizeTerminalSurfaceRequest(
        serializedBytes: request.message ?? Data())
      XCTAssertEqual(resize.owner.workspaceTerminal.workspacePath, "/repo")
      XCTAssertEqual(resize.owner.workspaceTerminal.terminalID, "sized")
    }
    let initial = try Releash_Client_V1_ResizeTerminalSurfaceRequest(
      serializedBytes: http.requests("ResizeTerminalSurface").last?.message ?? Data())
    XCTAssertEqual(initial.cols, 120)
    XCTAssertEqual(initial.rows, 40)
    var received = Releash_Client_V1_TerminalEvent()
    received.resize.cols = 90
    received.resize.rows = 25
    session.receive(received)
    await Task.yield()
    XCTAssertEqual(http.requests("ResizeTerminalSurface").count, count)
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
    model.confirmTerminalClose = {
      confirmations += 1
      return approved
    }
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
    let kill = try Releash_Client_V1_KillTerminalSurfaceRequest(
      serializedBytes: http.requests("KillTerminalSurface").last?.message ?? Data())
    XCTAssertEqual(kill.owner, session.owner)
    await model.stop()
  }

  @MainActor func testSnapshotWithTheDisplayedSizeDoesNotResizeTheTerminalSurface() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let session = model.terminal(path: "/repo", id: "same-size", agent: true)
    try await eventually { session.hasForegroundProcess != nil }
    session.input("first")
    try await eventually { http.requests("WriteTerminalSurface").count == 1 }
    XCTAssertEqual(http.requests("ResizeTerminalSurface").count, 0)

    // When
    var resent = Releash_Client_V1_TerminalEvent()
    resent.snapshot.cols = UIStyle.terminalColumns
    resent.snapshot.rows = UIStyle.terminalRows
    resent.snapshot.processedReportUnits = 5000
    session.receive(resent)
    session.input("second")
    try await eventually { http.requests("WriteTerminalSurface").count == 2 }

    // Then
    XCTAssertEqual(http.requests("ResizeTerminalSurface").count, 0)
    session.stop()
    await model.stop()
  }

  @MainActor func testOnlyDisplayChangesAndNewAttachmentsReportTheDisplayedSize() async throws {
    // Given: 表示領域が 125x78 に変わり、PTY へ一度だけ報告している
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let session = model.terminal(path: "/repo", id: "report-size", agent: true)
    try await eventually { session.hasForegroundProcess != nil }
    session.sizeChanged(source: session.screen, newCols: 125, newRows: 78)
    try await eventually { http.requests("ResizeTerminalSurface").count == 1 }

    // When: 同じ接続のまま、別の寸法の snapshot が送り直される
    var resent = Releash_Client_V1_TerminalEvent()
    resent.snapshot.replay = "resent"
    resent.snapshot.cols = 100
    resent.snapshot.rows = 40
    resent.snapshot.processedReportUnits = 5000
    session.receive(resent)
    session.input("after resend")
    try await eventually { http.requests("WriteTerminalSurface").count == 1 }

    // Then: 送り直しには応答しない
    XCTAssertEqual(http.requests("ResizeTerminalSurface").count, 1)

    // When: 接続を作り直し、新しい接続の最初の snapshot（80x24）が届く
    model.restartTerminalSubscription(path: "/repo", id: "report-size", agent: true)
    try await eventually { http.requests("ResizeTerminalSurface").count == 2 }

    // Then: 表示領域の寸法を一度だけ報告する
    let request = try Releash_Client_V1_ResizeTerminalSurfaceRequest(
      serializedBytes: XCTUnwrap(http.requests("ResizeTerminalSurface").last?.message))
    XCTAssertEqual(request.cols, 125)
    XCTAssertEqual(request.rows, 78)
    session.input("after reattach")
    try await eventually { http.requests("WriteTerminalSurface").count == 2 }
    XCTAssertEqual(http.requests("ResizeTerminalSurface").count, 2)
    session.stop()
    await model.stop()
  }

  @MainActor func testLinkDelegateRequestsTheDisplayedURLWithoutOpeningAnApp() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    var opened: [URL] = []
    let session = TerminalSession(
      model: model, path: "/repo", id: "link", openLink: { opened.append($0) })
    let allowed = [
      "http://example.com", "https://example.com/path?q=terminal", "mailto:dev@example.com",
      "tel:+123456789", "HTTPS://example.com",
    ]
    for link in allowed { session.requestOpenLink(source: session.screen, link: link, params: [:]) }
    XCTAssertEqual(opened, allowed.map { URL(string: $0)! })
    for link in ["file:///System/Applications/Calculator.app", "releash-custom://run", "/relative"]
    {
      session.requestOpenLink(source: session.screen, link: link, params: [:])
    }
    XCTAssertEqual(opened.count, allowed.count)
    session.stop()
    await model.stop()
  }

}

extension TerminalSessionTests {

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
      .split(
        id: "split", axis: .horizontal, ratio: 0.5,
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
      added = try XCTUnwrap(
        model.panes.controller.tabs(inPane: pane).compactMap { model.panes.tab($0.id) }.first {
          $0.id != first.id
        })
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
      XCTAssertEqual(
        Set(model.panes.controller.tabs(inPane: pane).compactMap { model.panes.tab($0.id)?.id }),
        Set([added.id]))
      XCTAssertEqual(http.requests("KillTerminalSurface").count, 1)
      model.confirmTerminalClose = { true }
      model.panes.controller.closePane(pane)
      try await eventually { !model.panes.controller.allPaneIds.contains(pane) }
      let kills = try http.requests("KillTerminalSurface").map {
        try Releash_Client_V1_KillTerminalSurfaceRequest(serializedBytes: $0.message ?? Data())
          .owner.workspaceTerminal.terminalID
      }
      XCTAssertEqual(kills, [first.id, added.id])
    } else {
      try await eventually { !model.panes.controller.allPaneIds.contains(pane) }
      let kills = try http.requests("KillTerminalSurface").map {
        try Releash_Client_V1_KillTerminalSurfaceRequest(serializedBytes: $0.message ?? Data())
          .owner.workspaceTerminal.terminalID
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
  @MainActor func testPaneCloseCancellationRemovesStoppedTabAndCanCloseRemainingPane() async throws
  {
    try await closePaneWithChangingTabs(move: false, cancel: true)
  }
}

extension TerminalSessionTests {
  @MainActor func testExitedSnapshotAndExitEventCloseWithoutConfirmationAfterReconnect()
    async throws
  {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectedWorktree = "/repo"
    model.confirmTerminalClose = {
      XCTFail("Exited terminal must not require confirmation")
      return false
    }
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
      try Releash_Client_V1_KillTerminalSurfaceRequest(serializedBytes: $0.message ?? Data()).owner
        .workspaceTerminal.terminalID
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
    let write = try Releash_Client_V1_WriteTerminalSurfaceRequest(
      serializedBytes: XCTUnwrap(http.requests("WriteTerminalSurface").last?.message))
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
  @MainActor func testDeletedAgentSessionDoesNotBlockOtherSubscriptionsDuringRecovery() async throws
  {
    // Given
    let http = TestHTTPClient()
    var tab = Releash_Client_V1_SessionTabSnapshot()
    tab.session.id = "deleted"
    http.state.withLock { $0.sessionTabs["deleted"] = tab }
    let model = try await testModel(http)
    var receivedSession = false
    var receivedDeletion = false
    let subscription = model.subscribe("session-tab", ["deleted"]) { payload in
      if case .sessionTab(let value) = payload.value {
        receivedSession = value.hasSession
        receivedDeletion = !value.hasSession
      }
    }
    let terminal = model.terminal(path: "/repo", id: "deleted", agent: true)
    try await eventually { receivedSession && terminal.hasForegroundProcess != nil }
    let previous = try XCTUnwrap(
      model.terminalAttachment(path: "/repo", id: "deleted", agent: true))
    let stream = http.state.withLock { state in
      state.sessionTabs.removeValue(forKey: "deleted")
      state.failedMethod = "OpenAgentSession"
      var repo = Releash_Client_V1_WorkspaceRepositoryList()
      repo.path = "/recovered"
      state.repositories = [repo]
      return state.streams.last!
    }
    // When
    stream.receiveClose(
      .unavailable, [:], ConnectError(code: .unavailable, message: "disconnected"))
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

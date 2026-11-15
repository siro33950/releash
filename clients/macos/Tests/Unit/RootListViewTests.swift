import AppKit
import ApplicationServices
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class RootListViewTests: XCTestCase {
  @MainActor func testEachRootOpensItsKindAndPendingWorkflowWaitsForItsRoot() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    var session = Releash_Client_V1_WorkspaceTreeItem()
    session.node.id = "session-tree"
    session.node.sessionID = "agent-session"
    session.node.title = "Session"
    session.node.contentKind.value = .session
    var command = Releash_Client_V1_WorkspaceTreeItem()
    command.node.id = "command"
    command.node.title = "Command"
    command.node.contentKind.value = .command
    var sequence = Releash_Client_V1_WorkspaceTreeItem()
    sequence.sequence.id = "sequence"
    sequence.sequence.title = "Sequence"
    var fanout = Releash_Client_V1_WorkspaceTreeItem()
    fanout.fanout.id = "fanout"
    fanout.fanout.title = "Fanout"
    for root in [session, command, sequence, fanout] { model.openRoot(root) }
    XCTAssertEqual(try model.panes.layout().tabs.map(\.kind), [.session, .command])
    XCTAssertEqual(try model.panes.layout().tabs.map(\.nodeID), [nil, "command"])
    XCTAssertEqual(try model.panes.layout().tabs.first?.agentSessionID, "agent-session")
    model.pendingRoots["/repo"] = "pending"
    model.openPendingRoot()
    XCTAssertEqual(model.pendingRoots["/repo"], "pending")
    sequence.sequence.id = "pending"
    var tree = Releash_Client_V1_WorkspaceWorktreeList()
    tree.path = "/repo"
    tree.snapshot.nodes.items = [sequence]
    var repo = Releash_Client_V1_WorkspaceRepositoryList()
    repo.worktrees.items = [tree]
    model.repositories = [repo]
    model.layoutLoaded = false
    model.openPendingRoot()
    XCTAssertEqual(model.pendingRoots["/repo"], "pending")
    model.layoutLoaded = true
    model.openPendingRoot()
    XCTAssertNil(model.pendingRoots["/repo"])
    XCTAssertEqual(try model.panes.layout().tabs.last?.nodeID, "command")
    await model.stop()
  }

  @MainActor func testSelectionChangeOpensTabOnlyForACurrentMouseClick() async throws {
    func mouse(_ type: NSEvent.EventType, at timestamp: TimeInterval) throws -> NSEvent {
      try XCTUnwrap(
        NSEvent.mouseEvent(
          with: type, location: .zero, modifierFlags: [], timestamp: timestamp, windowNumber: 0,
          context: nil, eventNumber: 0, clickCount: 1, pressure: 1))
    }
    let key = try XCTUnwrap(
      NSEvent.keyEvent(
        with: .keyDown, location: .zero, modifierFlags: [], timestamp: 100, windowNumber: 0,
        context: nil, characters: "", charactersIgnoringModifiers: "", isARepeat: false,
        keyCode: 125))
    XCTAssertTrue(RootListView.isClick(try mouse(.leftMouseDown, at: 100), now: 100.01))
    XCTAssertTrue(RootListView.isClick(try mouse(.leftMouseUp, at: 100), now: 100.5))
    XCTAssertFalse(RootListView.isClick(try mouse(.leftMouseDown, at: 100), now: 105))
    XCTAssertFalse(RootListView.isClick(try mouse(.rightMouseDown, at: 100), now: 100.01))
    XCTAssertFalse(RootListView.isClick(key, now: 100.01))
    XCTAssertFalse(RootListView.isClick(nil, now: 100))

    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    var command = Releash_Client_V1_WorkspaceTreeItem()
    command.node.id = "command"
    command.node.title = "Command"
    command.node.contentKind.value = .command
    var tree = Releash_Client_V1_WorkspaceWorktreeList()
    tree.path = "/repo"
    tree.snapshot.nodes.items = [command]
    var repo = Releash_Client_V1_WorkspaceRepositoryList()
    repo.worktrees.items = [tree]
    model.repositories = [repo]
    let view = RootListView(model: model, path: "/repo")
    view.selectionChanged(to: "command", by: key, now: 100.01)
    view.selectionChanged(to: "command", by: try mouse(.leftMouseDown, at: 100), now: 105)
    view.selectionChanged(to: "missing", by: try mouse(.leftMouseDown, at: 100), now: 100.01)
    XCTAssertTrue(try model.panes.layout().tabs.isEmpty)
    view.selectionChanged(to: "command", by: try mouse(.leftMouseDown, at: 100), now: 100.01)
    XCTAssertEqual(try model.panes.layout().tabs.map(\.nodeID), ["command"])
    await model.stop()
  }

  @MainActor func testDetailHeightStartsAtOneThirdAndKeepsBothPanesAboveTheirMinimums() {
    let tree = UIStyle.nodeTreeMinimumHeight
    let detail = UIStyle.nodeDetailMinimumHeight
    XCTAssertEqual(RootListView.detailHeight(total: 900, chosen: nil), 300)
    XCTAssertEqual(RootListView.detailHeight(total: 900, chosen: 420), 420)
    XCTAssertEqual(RootListView.detailHeight(total: 900, chosen: 10), detail)
    XCTAssertEqual(RootListView.detailHeight(total: 900, chosen: 5000), 900 - tree)
    XCTAssertEqual(RootListView.detailHeight(total: 400, chosen: nil), detail)
    XCTAssertEqual(RootListView.detailHeight(total: tree + 40, chosen: nil), 40)
    XCTAssertEqual(RootListView.detailHeight(total: 60, chosen: nil), 0)
  }

  @MainActor func testArchiveRetryResumeAndAbortUseServerTargetsAndReportFailure() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    var root = Releash_Client_V1_WorkspaceTreeItem()
    root.sequence.id = "execution"
    RootListView(model: model, path: "/repo").archiveRoot(root.rootID)
    model.resumeNode(path: "/repo", id: "session-node")
    var detail = Releash_Client_V1_WorkspaceNodeDetailDto()
    detail.id = "command-node"
    detail.executionID = "execution"
    model.retryNode(path: "/repo", id: detail.id)
    model.abortExecution(id: detail.executionID)
    try await eventually {
      !http.requests("ArchiveWorkspaceWorkflowExecution").isEmpty
        && !http.requests("RetryWorkspaceNode").isEmpty
        && !http.requests("ResumeWorkspaceSessionNode").isEmpty
        && !http.requests("AbortWorkflow").isEmpty
    }
    let archive = try Releash_Client_V1_ArchiveWorkspaceWorkflowExecutionRequest(
      serializedBytes: XCTUnwrap(http.requests("ArchiveWorkspaceWorkflowExecution").first?.message))
    XCTAssertEqual(archive.executionID, "execution")
    XCTAssertEqual(archive.worktreePath, "/repo")
    let retry = try Releash_Client_V1_RetryWorkspaceNodeRequest(
      serializedBytes: XCTUnwrap(http.requests("RetryWorkspaceNode").first?.message))
    XCTAssertEqual(retry.nodeID, "command-node")
    XCTAssertEqual(retry.worktreePath, "/repo")
    let resume = try Releash_Client_V1_ResumeWorkspaceSessionNodeRequest(
      serializedBytes: XCTUnwrap(http.requests("ResumeWorkspaceSessionNode").first?.message))
    XCTAssertEqual(resume.nodeID, "session-node")
    let abort = try Releash_Client_V1_AbortWorkflowRequest(
      serializedBytes: XCTUnwrap(http.requests("AbortWorkflow").first?.message))
    XCTAssertEqual(abort.executionID, "execution")
    http.state.withLock { $0.failedMethod = "AbortWorkflow" }
    model.abortExecution(id: detail.executionID)
    try await eventually { model.failure == "AbortWorkflow failed" }
    await model.stop()
  }

}

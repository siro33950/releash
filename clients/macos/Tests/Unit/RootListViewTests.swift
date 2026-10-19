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

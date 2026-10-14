import AppKit
import SwiftUI
import SwiftProtobuf
import XCTest

@testable import Releash

final class RootListViewTests: XCTestCase {
  @MainActor func testEachRootOpensItsKindAndPendingWorkflowWaitsForItsRoot() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    var session = Releash_Client_V1_WorkspaceTreeItem()
    session.node.id = "session"; session.node.title = "Session"; session.node.contentKind.value = .session
    var command = Releash_Client_V1_WorkspaceTreeItem()
    command.node.id = "command"; command.node.title = "Command"; command.node.contentKind.value = .command
    var sequence = Releash_Client_V1_WorkspaceTreeItem()
    sequence.sequence.id = "sequence"; sequence.sequence.title = "Sequence"
    var fanout = Releash_Client_V1_WorkspaceTreeItem()
    fanout.fanout.id = "fanout"; fanout.fanout.title = "Fanout"
    for root in [session, command, sequence, fanout] { model.openRoot(root) }
    XCTAssertEqual(try model.panes.layout().tabs.map(\.kind), [.session, .command, .workflow, .workflow])
    XCTAssertEqual(try model.panes.layout().tabs.map(\.nodeID), ["session", "command", "sequence", "fanout"])
    model.pendingRoots["/repo"] = "pending"
    model.openPendingRoot()
    XCTAssertEqual(model.pendingRoots["/repo"], "pending")
    sequence.sequence.id = "pending"
    var tree = Releash_Client_V1_WorkspaceWorktreeList()
    tree.path = "/repo"; tree.snapshot.nodes.items = [sequence]
    var repo = Releash_Client_V1_WorkspaceRepositoryList()
    repo.worktrees.items = [tree]; model.repositories = [repo]
    model.layoutLoaded = false
    model.openPendingRoot()
    XCTAssertEqual(model.pendingRoots["/repo"], "pending")
    model.layoutLoaded = true
    model.openPendingRoot()
    XCTAssertNil(model.pendingRoots["/repo"])
    XCTAssertEqual(try model.panes.layout().tabs.last?.nodeID, "pending")
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
    detail.id = "command-node"; detail.executionID = "execution"
    let pane = CommandPane(model: model, path: "/repo", detail: detail, command: .init())
    pane.retry(); pane.abort()
    try await eventually { !http.requests("ArchiveWorkspaceWorkflowExecution").isEmpty && !http.requests("RetryWorkspaceNode").isEmpty && !http.requests("ResumeWorkspaceSessionNode").isEmpty && !http.requests("AbortWorkflow").isEmpty }
    let archive = try Releash_Client_V1_ArchiveWorkspaceWorkflowExecutionRequest(serializedBytes: XCTUnwrap(http.requests("ArchiveWorkspaceWorkflowExecution").first?.message))
    XCTAssertEqual(archive.executionID, "execution"); XCTAssertEqual(archive.worktreePath, "/repo")
    let retry = try Releash_Client_V1_RetryWorkspaceNodeRequest(serializedBytes: XCTUnwrap(http.requests("RetryWorkspaceNode").first?.message))
    XCTAssertEqual(retry.nodeID, "command-node"); XCTAssertEqual(retry.worktreePath, "/repo")
    let resume = try Releash_Client_V1_ResumeWorkspaceSessionNodeRequest(serializedBytes: XCTUnwrap(http.requests("ResumeWorkspaceSessionNode").first?.message))
    XCTAssertEqual(resume.nodeID, "session-node")
    let abort = try Releash_Client_V1_AbortWorkflowRequest(serializedBytes: XCTUnwrap(http.requests("AbortWorkflow").first?.message))
    XCTAssertEqual(abort.executionID, "execution")
    http.state.withLock { $0.failedMethod = "AbortWorkflow" }
    pane.abort()
    try await eventually { model.failure == "AbortWorkflow failed" }
    await model.stop()
  }
  @MainActor func testPendingWorkflowBelongsToStartingWorktreeAcrossSelectionChanges() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/a"); try await eventually { model.layoutLoaded }
    let pane = try XCTUnwrap(model.panes.controller.focusedPaneId)
    try await PaneLaunchView(model: model, pane: pane, path: "/a", kind: .workflow).startWorkflow(name: "flow", requestText: "")
    XCTAssertEqual(model.pendingRoots["/a"], "workflow-execution")
    model.selectWorktree("/b"); try await eventually { model.layoutLoaded }
    XCTAssertNil(model.pendingRoots["/b"])
    let host = await ViewHarness(RootListView(model: model, path: "/b"))
    try await eventuallyView(host) { host.element("New Workflow") != nil }
    XCTAssertTrue(host.element("New Workflow")?.enabled == true)
    XCTAssertNil(host.element("Rootの配信を待っています"))
    var root = Releash_Client_V1_WorkspaceTreeItem(); root.node.id = "workflow-execution"; root.node.title = "A command"; root.node.contentKind.value = .command
    var a = Releash_Client_V1_WorkspaceWorktreeList(); a.path = "/a"; a.snapshot.nodes.items = [root]
    var b = Releash_Client_V1_WorkspaceWorktreeList(); b.path = "/b"
    var repo = Releash_Client_V1_WorkspaceRepositoryList(); repo.worktrees.items = [a, b]
    var payload = Releash_Client_V1_StatePayload(); payload.workspaces.repositories.items = [repo]
    try http.publish(payload, target: "workspaces")
    try await eventually { model.repositories.first?.worktrees.items.count == 2 }
    XCTAssertTrue(model.panes.controller.allTabIds.isEmpty)
    XCTAssertEqual(model.pendingRoots["/a"], "workflow-execution")
    try await PaneLaunchView(model: model, pane: try XCTUnwrap(model.panes.controller.focusedPaneId), path: "/b", kind: .workflow).startWorkflow(name: "another", requestText: "")
    XCTAssertEqual(model.pendingRoots["/b"], "workflow-execution")
    host.close()
    model.deselectWorktree("/b"); XCTAssertEqual(model.pendingRoots.count, 2)
    model.selectWorktree("/a"); try await eventually { model.layoutLoaded && model.pendingRoots["/a"] == nil }
    XCTAssertEqual(try model.panes.layout().tabs.last?.nodeID, "workflow-execution")
    XCTAssertEqual(try model.panes.layout().tabs.last?.kind, .command)
    XCTAssertEqual(model.pendingRoots["/b"], "workflow-execution")
    await model.stop()
  }

  @MainActor func testRootRowsMenusArchiveConfirmationAndRenameSubscription() async throws {
    let http = TestHTTPClient()
    var session = Releash_Client_V1_WorkspaceTreeItem(); session.node.id = "session-root"; session.node.title = "Original title"; session.node.contentKind.value = .session; session.node.provider.value = .claude; session.node.status.value = .active
    var command = Releash_Client_V1_WorkspaceTreeItem(); command.node.id = "command-root"; command.node.title = "Command title"; command.node.contentKind.value = .command; command.node.status.value = .attention
    var sequence = Releash_Client_V1_WorkspaceTreeItem(); sequence.sequence.id = "sequence-root"; sequence.sequence.title = "Sequence title"; sequence.sequence.status.value = .idle
    var fanout = Releash_Client_V1_WorkspaceTreeItem(); fanout.fanout.id = "fanout-root"; fanout.fanout.title = "Fanout title"; fanout.fanout.status.value = .active
    var tree = Releash_Client_V1_WorkspaceWorktreeList(); tree.path = "/repo"; tree.snapshot.nodes.items = [session, command, sequence, fanout]
    var repo = Releash_Client_V1_WorkspaceRepositoryList(); repo.path = "/repo"; repo.worktrees.items = [tree]
    http.state.withLock { $0.repositories = [repo] }
    let model = try await testModel(http)
    model.selectWorktree("/repo"); try await eventually { model.layoutLoaded }
    let host = await ViewHarness(RootListView(model: model, path: "/repo"))
    try await eventuallyView(host) { host.element("root:session-root") != nil }
    for (id, status, title) in [("session-root", "active", "Original title"), ("command-root", "attention", "Command title"), ("sequence-root", "idle", "Sequence title"), ("fanout-root", "active", "Fanout title")] {
      let element = try XCTUnwrap(host.element("root:\(id)"))
      XCTAssertEqual(host.value(element), status)
      XCTAssertTrue(element.label.contains(title))
    }
    XCTAssertEqual(command.rootIcon, "terminal")
    XCTAssertEqual(sequence.rootIcon, "arrow.right")
    XCTAssertEqual(fanout.rootIcon, "arrow.triangle.branch")
    let sessionLabel = try XCTUnwrap(host.element("root:session-root")).label
    XCTAssertLessThan(try XCTUnwrap(sessionLabel.range(of: "Claude")).lowerBound, try XCTUnwrap(sessionLabel.range(of: "Original title")).lowerBound)
    XCTAssertTrue(host.element("root:session-root")?.label.contains("Claude") == true)
    XCTAssertTrue(host.element("root:session-root")?.label.contains("Session") == true)
    XCTAssertTrue(host.element("root:command-root")?.label.contains("Command") == true)
    try await host.press("root:session-root")
    XCTAssertEqual(try model.panes.layout().tabs.last?.nodeID, "session-root")
    try await host.menu("root:command-root", selecting: nil) { texts in
      for action in ["改名", "Resume", "Archive"] { XCTAssertFalse(texts.contains(action)) }
    }
    session.node.capabilities.canRename = true; session.node.capabilities.canResumeSession = true; session.node.workflowCapabilities.canArchive = true; session.node.workflowCapabilities.canAbort = true
    tree.snapshot.nodes.items[0] = session; repo.worktrees.items = [tree]
    var payload = Releash_Client_V1_StatePayload(); payload.workspaces.repositories.items = [repo]
    try http.publish(payload, target: "workspaces")
    try await Task.sleep(for: .milliseconds(100))
    try await host.menu("root:session-root", selecting: "Archive") { texts in
      for action in ["改名", "Resume", "Archive"] { XCTAssertTrue(texts.contains(action)) }
    }
    try await eventuallyView(host) { host.element("キャンセル") != nil }
    XCTAssertTrue(http.requests("ArchiveWorkspaceWorkflowExecution").isEmpty)
    try await host.pressAlertButton("キャンセル")
    XCTAssertTrue(http.requests("ArchiveWorkspaceWorkflowExecution").isEmpty)
    try await host.menu("root:session-root", selecting: "Archive") { XCTAssertTrue($0.contains("Archive")) }
    try await eventuallyView(host) { host.element("中止してArchive") != nil }; try await host.pressAlertButton("中止してArchive")
    try await eventually { http.requests("ArchiveWorkspaceWorkflowExecution").count == 1 }
    try await host.menu("root:session-root", selecting: "改名") { XCTAssertTrue($0.contains("改名")) }
    try await eventuallyView(host) { host.element("保存") != nil }
    try await host.setTextField("Renamed title")
    try await host.pressAlertButton("保存")
    try await eventually { !http.requests("RenameWorkspaceSessionNode").isEmpty }
    let request = try Releash_Client_V1_RenameWorkspaceSessionNodeRequest(serializedBytes: XCTUnwrap(http.requests("RenameWorkspaceSessionNode").last?.message))
    XCTAssertEqual(request.nodeID, "session-root"); XCTAssertEqual(request.name, "Renamed title")
    session.node.title = "Renamed title"; session.node.workflowCapabilities.canAbort = false
    tree.snapshot.nodes.items[0] = session; repo.worktrees.items = [tree]; payload.workspaces.repositories.items = [repo]
    try http.publish(payload, target: "workspaces")
    try await eventuallyView(host) { model.panes.controller.allPaneIds.flatMap { model.panes.controller.tabs(inPane: $0) }.last?.title == "Renamed title" && host.element("root:session-root")?.label.contains("Renamed title") == true }
    try await host.menu("root:session-root", selecting: "Archive") { XCTAssertTrue($0.contains("Archive")) }
    try await eventually { http.requests("ArchiveWorkspaceWorkflowExecution").count == 2 }
    XCTAssertNil(host.element("中止してArchive"))
    host.close(); await model.stop()
  }

}

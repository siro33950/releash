import AppKit
import ApplicationServices
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class RootListViewTests: XCTestCase {
  @MainActor func testPendingWorkflowBelongsToStartingWorktreeAcrossSelectionChanges() async throws
  {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/a")
    try await eventually { model.layoutLoaded }
    try await model.startWorkflow(name: "flow", path: "/a")
    XCTAssertEqual(model.pendingRoots["/a"], "workflow-execution")
    model.selectWorktree("/b")
    try await eventually { model.layoutLoaded }
    XCTAssertNil(model.pendingRoots["/b"])
    let host = await ViewHarness(RootListView(model: model, path: "/b"))
    try await eventuallyView(host) { host.element("開始") != nil }
    XCTAssertNil(host.element("Rootの配信を待っています"))
    var root = Releash_Client_V1_WorkspaceTreeItem()
    root.node.id = "workflow-execution"
    root.node.title = "A command"
    root.node.contentKind.value = .command
    var a = Releash_Client_V1_WorkspaceWorktreeList()
    a.path = "/a"
    a.snapshot.nodes.items = [root]
    var b = Releash_Client_V1_WorkspaceWorktreeList()
    b.path = "/b"
    var repo = Releash_Client_V1_WorkspaceRepositoryList()
    repo.worktrees.items = [a, b]
    var payload = Releash_Client_V1_StatePayload()
    payload.workspaces.repositories.items = [repo]
    try http.publish(payload, target: "workspaces")
    try await eventually { model.repositories.first?.worktrees.items.count == 2 }
    XCTAssertTrue(model.panes.controller.allTabIds.isEmpty)
    XCTAssertEqual(model.pendingRoots["/a"], "workflow-execution")
    try await model.startWorkflow(name: "another", path: "/b")
    XCTAssertEqual(model.pendingRoots["/b"], "workflow-execution")
    host.close()
    model.deselectWorktree("/b")
    XCTAssertEqual(model.pendingRoots.count, 2)
    model.selectWorktree("/a")
    try await eventually { model.layoutLoaded && model.pendingRoots["/a"] == nil }
    XCTAssertEqual(try model.panes.layout().tabs.last?.nodeID, "workflow-execution")
    XCTAssertEqual(try model.panes.layout().tabs.last?.kind, .command)
    XCTAssertEqual(model.pendingRoots["/b"], "workflow-execution")
    await model.stop()
  }

  @MainActor func testRootRowsMenusArchiveConfirmationAndRenameSubscription() async throws {
    let http = TestHTTPClient()
    var session = Releash_Client_V1_WorkspaceTreeItem()
    session.node.id = "session-root"
    session.node.title = "Original title"
    session.node.contentKind.value = .session
    session.node.provider.value = .claude
    session.node.status.value = .active
    var command = Releash_Client_V1_WorkspaceTreeItem()
    command.node.id = "command-root"
    command.node.title = "Command title"
    command.node.contentKind.value = .command
    command.node.status.value = .attention
    var sequence = Releash_Client_V1_WorkspaceTreeItem()
    sequence.sequence.id = "sequence-root"
    sequence.sequence.title = "Sequence title"
    sequence.sequence.status.value = .idle
    var fanout = Releash_Client_V1_WorkspaceTreeItem()
    fanout.fanout.id = "fanout-root"
    fanout.fanout.title = "Fanout title"
    fanout.fanout.status.value = .active
    var tree = Releash_Client_V1_WorkspaceWorktreeList()
    tree.path = "/repo"
    tree.snapshot.nodes.items = [session, command, sequence, fanout]
    var repo = Releash_Client_V1_WorkspaceRepositoryList()
    repo.path = "/repo"
    repo.worktrees.items = [tree]
    http.state.withLock { $0.repositories = [repo] }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    let host = await ViewHarness(RootListView(model: model, path: "/repo"))
    try await eventuallyView(host) { host.element("root:session-root") != nil }
    for (id, status, title) in [
      ("session-root", "active", "Original title"), ("command-root", "attention", "Command title"),
      ("sequence-root", "idle", "Sequence title"), ("fanout-root", "active", "Fanout title"),
    ] {
      let element = try XCTUnwrap(host.element("root:\(id)"))
      XCTAssertEqual(host.value(element), status)
      XCTAssertTrue(element.label.contains(title))
    }
    let sessionLabel = try XCTUnwrap(host.element("root:session-root")).label
    XCTAssertLessThan(
      try XCTUnwrap(sessionLabel.range(of: "Claude")).lowerBound,
      try XCTUnwrap(sessionLabel.range(of: "Original title")).lowerBound)
    XCTAssertTrue(host.element("root:session-root")?.label.contains("Claude") == true)
    XCTAssertTrue(host.element("root:session-root")?.label.contains("Session") == true)
    XCTAssertTrue(host.element("root:command-root")?.label.contains("Command") == true)
    try await host.selectRow("root:session-root")
    XCTAssertTrue(model.panes.controller.allTabIds.isEmpty)
    try await host.press("open-node:session-root")
    XCTAssertEqual(try model.panes.layout().tabs.last?.nodeID, "session-root")
    try await host.menu("root:command-root", selecting: nil) { texts in
      for action in ["Rename", "Resume", "Archive"] { XCTAssertFalse(texts.contains(action)) }
    }
    session.node.capabilities.canRename = true
    session.node.capabilities.canResumeSession = true
    session.node.workflowCapabilities.canArchive = true
    session.node.workflowCapabilities.canAbort = true
    tree.snapshot.nodes.items[0] = session
    repo.worktrees.items = [tree]
    var payload = Releash_Client_V1_StatePayload()
    payload.workspaces.repositories.items = [repo]
    try http.publish(payload, target: "workspaces")
    try await eventually {
      guard
        case .node(let root) = model.repositories.first?.worktrees.items.first?.snapshot.nodes.items
          .first?.variant
      else { return false }
      return root.capabilities.canRename
    }
    try await host.menu("root:session-root", selecting: "Archive") { texts in
      for action in ["Rename", "Resume", "Archive"] { XCTAssertTrue(texts.contains(action)) }
    }
    try await eventuallyView(host) { host.element("キャンセル") != nil }
    XCTAssertTrue(http.requests("ArchiveWorkspaceWorkflowExecution").isEmpty)
    try await host.pressAlertButton("キャンセル")
    XCTAssertTrue(http.requests("ArchiveWorkspaceWorkflowExecution").isEmpty)
    try await host.menu("root:session-root", selecting: "Archive") {
      XCTAssertTrue($0.contains("Archive"))
    }
    try await eventuallyView(host) { host.element("Abort and Archive") != nil }
    try await host.pressAlertButton("Abort and Archive")
    try await eventually { http.requests("ArchiveWorkspaceWorkflowExecution").count == 1 }
    try await host.menu("root:session-root", selecting: "Rename") {
      XCTAssertTrue($0.contains("Rename"))
    }
    try await eventuallyView(host) { host.element("保存") != nil }
    try await host.setTextField("Renamed title")
    try await host.pressAlertButton("保存")
    try await eventually { !http.requests("RenameWorkspaceSessionNode").isEmpty }
    let request = try Releash_Client_V1_RenameWorkspaceSessionNodeRequest(
      serializedBytes: XCTUnwrap(http.requests("RenameWorkspaceSessionNode").last?.message))
    XCTAssertEqual(request.nodeID, "session-root")
    XCTAssertEqual(request.name, "Renamed title")
    session.node.title = "Renamed title"
    session.node.workflowCapabilities.canAbort = false
    tree.snapshot.nodes.items[0] = session
    repo.worktrees.items = [tree]
    payload.workspaces.repositories.items = [repo]
    try http.publish(payload, target: "workspaces")
    try await eventuallyView(host) {
      model.panes.controller.allPaneIds.flatMap { model.panes.controller.tabs(inPane: $0) }.last?
        .title == "Claude Renamed title"
        && host.element("root:session-root")?.label.contains("Renamed title") == true
    }
    try await host.menu("root:session-root", selecting: "Archive") {
      XCTAssertTrue($0.contains("Archive"))
    }
    try await eventually { http.requests("ArchiveWorkspaceWorkflowExecution").count == 2 }
    XCTAssertNil(host.element("Abort and Archive"))
    host.close()
    await model.stop()
  }

  @MainActor func testRowsShowKindsFailuresModelsArtifactsAndExpandPastAttempts() async throws {
    let http = TestHTTPClient()
    var session = Releash_Client_V1_WorkspaceTreeItem()
    session.node.id = "specified-session"
    session.node.title = "Specified session"
    session.node.contentKind.value = .session
    session.node.model = "session-model"
    session.node.errorReason = "Session failed"
    session.node.hasArtifact_p = true
    var past = Releash_Client_V1_WorkspacePastAttempt()
    past.node.id = "past-session"
    past.node.title = "Previous session"
    past.node.contentKind.value = .session
    session.node.pastAttempts.items = [past]
    var unset = Releash_Client_V1_WorkspaceTreeItem()
    unset.node.id = "unset-session"
    unset.node.title = "Unset session"
    unset.node.contentKind.value = .session
    var command = Releash_Client_V1_WorkspaceTreeItem()
    command.node.id = "failed-command"
    command.node.title = "Failed command"
    command.node.contentKind.value = .command
    command.node.errorReason = "Command failed"
    var sequence = Releash_Client_V1_WorkspaceTreeItem()
    sequence.sequence.id = "isolated-sequence"
    sequence.sequence.title = "Sequence"
    sequence.sequence.worktree.branch = "sequence-branch"
    sequence.sequence.worktree.path = "/isolated/sequence"
    var fanout = Releash_Client_V1_WorkspaceTreeItem()
    fanout.fanout.id = "isolated-fanout"
    fanout.fanout.title = "Fanout"
    fanout.fanout.worktree.branch = "fanout-branch"
    fanout.fanout.worktree.path = "/isolated/fanout"
    var tree = Releash_Client_V1_WorkspaceWorktreeList()
    tree.path = "/repo"
    tree.snapshot.nodes.items = [session, unset, command, sequence, fanout]
    var repo = Releash_Client_V1_WorkspaceRepositoryList()
    repo.worktrees.items = [tree]
    http.state.withLock { $0.repositories = [repo] }
    let model = try await testModel(http)
    let host = await ViewHarness(RootListView(model: model, path: "/repo"))
    defer { host.close() }
    try await eventuallyView(host) { host.element("root:isolated-fanout") != nil }
    XCTAssertTrue(
      try XCTUnwrap(host.element("root:specified-session")).label.contains("Session failed"))
    XCTAssertTrue(
      try XCTUnwrap(host.element("root:failed-command")).label.contains("Command failed"))
    for (id, title, modelValue, artifact) in [
      ("specified-session", "Specified session", "session-model", "◆"),
      ("unset-session", "Unset session", "", ""),
      ("failed-command", "Failed command", "", ""),
    ] {
      XCTAssertEqual(
        try XCTUnwrap(host.element("node-model:\(id)")).label, "\(title)を選択、MODEL \(modelValue)")
      XCTAssertEqual(try XCTUnwrap(host.element("node-artifact:\(id)")).value, artifact)
    }
    for (id, kind, branch, path) in [
      ("isolated-sequence", "SEQ", "sequence-branch", "/isolated/sequence"),
      ("isolated-fanout", "FANOUT", "fanout-branch", "/isolated/fanout"),
    ] {
      let row = try XCTUnwrap(host.element("root:\(id)")).label
      XCTAssertTrue(row.contains(kind))
      XCTAssertTrue(row.contains(branch))
      XCTAssertTrue(row.contains(path))
    }
    XCTAssertNil(host.element("root:past-session"))
    try await host.press("過去の試行")
    try await eventuallyView(host) { host.element("root:past-session") != nil }
    XCTAssertTrue(
      try XCTUnwrap(host.element("root:past-session")).label.contains("Previous session"))
    try await host.press("過去の試行")
    try await eventuallyView(host) { host.element("root:past-session") == nil }
    await model.stop()
  }

  @MainActor func testDelegateChildrenAndPastAttemptsExpandIndependently() async throws {
    let http = TestHTTPClient()
    var session = Releash_Client_V1_WorkspaceTreeItem()
    session.node.id = "session"
    session.node.title = "Session"
    session.node.contentKind.value = .session
    var child = Releash_Client_V1_WorkspaceTreeItem()
    child.node.id = "delegate"
    child.node.title = "Delegate"
    child.node.contentKind.value = .command
    session.node.children.items = [child]
    var past = Releash_Client_V1_WorkspacePastAttempt()
    past.node.id = "past"
    past.node.title = "Past"
    past.node.contentKind.value = .session
    session.node.pastAttempts.items = [past]
    var tree = Releash_Client_V1_WorkspaceWorktreeList()
    tree.path = "/repo"
    tree.snapshot.nodes.items = [session]
    var repo = Releash_Client_V1_WorkspaceRepositoryList()
    repo.worktrees.items = [tree]
    http.state.withLock { $0.repositories = [repo] }
    let model = try await testModel(http)
    let host = await ViewHarness(RootListView(model: model, path: "/repo"))
    defer { host.close() }
    try await eventuallyView(host) { host.element("root:session") != nil }
    XCTAssertNil(host.element("root:delegate"))
    XCTAssertNil(host.element("root:past"))
    try await host.setRowExpanded("root:session", true)
    try await eventuallyView(host) { host.element("root:delegate") != nil }
    XCTAssertNil(host.element("root:past"))
    try await host.setRowExpanded("root:session", false)
    try await eventuallyView(host) { host.element("root:delegate") == nil }
    try await host.press("過去の試行")
    try await eventuallyView(host) { host.element("root:past") != nil }
    XCTAssertNil(host.element("root:delegate"))
    try await host.setRowExpanded("root:session", true)
    try await eventuallyView(host) { host.element("root:delegate") != nil }
    XCTAssertNotNil(host.element("root:past"))
    try await host.press("過去の試行")
    try await eventuallyView(host) { host.element("root:past") == nil }
    XCTAssertNotNil(host.element("root:delegate"))
    await model.stop()
  }

  @MainActor func testTreeExpandsParallelChildrenAndDeepReentriesAndApproveUsesNode() async throws {
    let http = TestHTTPClient()
    var root = Releash_Client_V1_WorkspaceTreeItem()
    root.sequence.id = "flow"
    root.sequence.title = "Flow"
    root.sequence.status.value = .attention
    root.sequence.canApprove = true
    var fanout = Releash_Client_V1_WorkspaceTreeItem()
    fanout.fanout.id = "parallel"
    fanout.fanout.title = "Parallel"
    fanout.fanout.status.value = .attention
    fanout.fanout.canApprove = true
    for index in 0..<12 {
      var leaf = Releash_Client_V1_WorkspaceTreeItem()
      leaf.node.id = "leaf-\(index)"
      leaf.node.title = "loop"
      leaf.node.nodeExecutionID = "visit-\(index)"
      leaf.node.contentKind.value = .session
      leaf.node.status.value = index == 0 ? .attention : .idle
      leaf.node.capabilities.canApprove = index == 0
      leaf.node.model = "test-model"
      leaf.node.hasArtifact_p = true
      fanout.fanout.children.items.append(leaf)
    }
    var deep = Releash_Client_V1_WorkspaceTreeItem()
    deep.node.id = "deep"
    deep.node.title = "Deep"
    deep.node.contentKind.value = .command
    for depth in (1...3).reversed() {
      var sequence = Releash_Client_V1_WorkspaceTreeItem()
      sequence.sequence.id = "depth-\(depth)"
      sequence.sequence.title = "Depth \(depth)"
      sequence.sequence.children.items = [deep]
      deep = sequence
    }
    root.sequence.children.items = [fanout, deep]
    var tree = Releash_Client_V1_WorkspaceWorktreeList()
    tree.path = "/repo"
    tree.snapshot.nodes.items = [root]
    var repo = Releash_Client_V1_WorkspaceRepositoryList()
    repo.worktrees.items = [tree]
    var detail = Releash_Client_V1_WorkspaceNodeDetailDto()
    detail.id = "leaf-0"
    detail.title = "loop"
    detail.nodeExecutionID = "visit-0"
    detail.nodeName = "loop"
    detail.model = "test-model"
    detail.content.session = .init()
    detail.classification.value = .attention
    var output = Releash_Client_V1_WorkflowOutputView()
    output.submitted.contract = "implementation_plan"
    output.submitted.structuredOutput.objectValue.entries["kind", default: .init()].stringValue
      .value = "implementation_plan"
    output.submitted.structuredOutput.objectValue.entries["node", default: .init()].stringValue
      .value = "loop"
    var task = Releash_Client_V1_WorkflowValue()
    task.objectValue.entries["id", default: .init()].stringValue.value = "fanout-child-key-anchor"
    task.objectValue.entries["problem", default: .init()].stringValue.value =
      "Observe each NodeExecution separately."
    output.submitted.structuredOutput.objectValue.entries["tasks", default: .init()].listValue
      .items = [task]
    http.state.withLock {
      $0.repositories = [repo]
      $0.nodeDetails["leaf-0"] = detail
      $0.artifacts["visit-0"] = output
    }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    let host = await ViewHarness(RootListView(model: model, path: "/repo"))
    host.window.setContentSize(NSSize(width: 540, height: 900))
    try await eventuallyView(host) { host.element("root:flow") != nil }
    try await host.selectRow("root:flow")
    try await eventuallyView(host) { host.element("root:parallel") != nil }
    try await host.selectRow("root:parallel")
    for index in 0..<12 {
      try await eventuallyView(host) { host.element("root:leaf-\(index)") != nil }
    }
    for depth in 1...3 {
      try await eventuallyView(host) { host.element("root:depth-\(depth)") != nil }
      try await host.selectRow("root:depth-\(depth)")
    }
    try await eventuallyView(host) { host.element("root:deep") != nil }
    try await host.selectRow("node-model:leaf-0")
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "node-detail" && $0.args == ["/repo", "leaf-0"] }
      }
    }
    try await eventuallyView(host) { host.texts().contains("implementation_plan") }
    let before = http.state.withLock { $0.subscriptions.count }
    try await host.selectRow("root:parallel")
    try await eventuallyView(host) { host.element("root:leaf-0") == nil }
    XCTAssertTrue(host.texts().contains("implementation_plan"))
    XCTAssertEqual(http.state.withLock { $0.subscriptions.count }, before)
    XCTAssertTrue(model.panes.controller.allTabIds.isEmpty)
    try await host.selectRow("root:parallel")
    try await eventuallyView(host) { host.element("approve:leaf-0") != nil }
    var selected: CFTypeRef?
    XCTAssertEqual(
      AXUIElementCopyAttributeValue(
        try host.tableRow("root:leaf-0"), kAXSelectedAttribute as CFString, &selected), .success)
    XCTAssertEqual(selected as? Bool, true)
    XCTAssertTrue(host.texts().contains("implementation_plan"))
    for id in ["flow", "parallel"] {
      XCTAssertNotNil(host.element("approve:" + id))
      let count = http.requests("ApproveWorkspaceNode").count
      try await host.press("approve:" + id)
      try await eventually { http.requests("ApproveWorkspaceNode").count == count + 1 }
      let request = try Releash_Client_V1_ApproveWorkspaceNodeRequest(
        serializedBytes: XCTUnwrap(http.requests("ApproveWorkspaceNode").last?.message))
      XCTAssertEqual(request.nodeID, id)
      XCTAssertEqual(request.worktreePath, "/repo")
    }
    XCTAssertNil(host.element("approve:depth-1"))
    let approveCount = http.requests("ApproveWorkspaceNode").count
    try await host.press("approve:leaf-0")
    try await eventually { http.requests("ApproveWorkspaceNode").count == approveCount + 1 }
    let request = try Releash_Client_V1_ApproveWorkspaceNodeRequest(
      serializedBytes: XCTUnwrap(http.requests("ApproveWorkspaceNode").last?.message))
    XCTAssertEqual(request.nodeID, "leaf-0")
    XCTAssertEqual(request.worktreePath, "/repo")
    XCTAssertNil(host.element("approve:leaf-1"))
    host.close()
    await model.stop()
  }
}

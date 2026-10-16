import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class SessionHistoryViewTests: XCTestCase {
  @MainActor func testHistoryActionsFollowResponsesAndSnapshotsAndKeepLoadedHistory() async throws {
    for method in [
      "RestoreAgentSession", "DeleteAgentSession", "ResumeAgentSessionHistoryCandidate",
    ] {
      let http = TestHTTPClient()
      var archived = Releash_Client_V1_AgentSessionItemDto()
      archived.id = "archived"
      archived.provider.value = .claude
      archived.providerSessionID = "archived-provider"
      archived.operations.canDelete = true
      var tree = Releash_Client_V1_WorkspaceWorktreeList()
      tree.path = "/repo"
      tree.snapshot.archivedSessions.items = [archived]
      var repo = Releash_Client_V1_WorkspaceRepositoryList()
      repo.path = "/repo"
      repo.worktrees.items = [tree]
      var candidate = Releash_Client_V1_AgentSessionHistoryCandidateDto()
      candidate.provider.value = .codex
      candidate.providerSessionID = "history-provider"
      candidate.label = "History session"
      var page = Releash_Client_V1_AgentSessionHistoryPageDto()
      page.items.items = [candidate]
      page.hasMore_p = true
      http.state.withLock {
        $0.repositories = [repo]
        $0.history = page
      }
      let model = try await testModel(http)
      model.selectWorktree("/repo")
      try await eventually { model.layoutLoaded }
      if method != "DeleteAgentSession" {
        model.openNode(.session, id: "agent-session", title: "Session")
      }
      let host = await ViewHarness(SessionHistoryView(model: model, path: "/repo"))
      try await eventuallyView(host) { host.element("Codex · History session") != nil }
      try await host.press("さらに読み込む")
      try await eventually {
        http.state.withLock {
          $0.subscriptions.contains {
            $0.target == "session-history" && $0.args == ["/repo", "40"]
          }
        }
      }
      XCTAssertNotNil(host.element("Codex · History session"))
      var older = candidate
      older.providerSessionID = "older"
      older.label = "Older session"
      page.items.items.append(older)
      page.hasMore_p = false
      var payload = Releash_Client_V1_StatePayload()
      payload.sessionHistory = page
      try http.publish(payload, target: "session-history")
      try await eventuallyView(host) { host.element("Codex · Older session") != nil }
      XCTAssertNotNil(host.element("Codex · History session"))
      XCTAssertNil(host.element("さらに読み込む"))
      let action =
        method == "DeleteAgentSession"
        ? "Delete"
        : method == "RestoreAgentSession" ? "Claude · archived-provider" : "Codex · History session"
      let tabsBeforeFailure = model.panes.controller.allTabIds
      http.state.withLock { $0.failedMethod = method }
      try await host.press(action)
      try await eventuallyView(host) { host.texts().contains("\(method) failed") }
      XCTAssertEqual(model.panes.controller.allTabIds, tabsBeforeFailure)
      XCTAssertNotNil(host.element("Claude · archived-provider"))
      http.state.withLock { $0.failedMethod = nil }
      try await host.press(action)
      try await eventually { http.requests(method).count == 2 }
      if method == "DeleteAgentSession" {
        XCTAssertTrue(model.panes.controller.allTabIds.isEmpty)
        let request = try Releash_Client_V1_DeleteAgentSessionRequest(
          serializedBytes: XCTUnwrap(http.requests(method).last?.message))
        XCTAssertEqual(request.agentSessionID, "archived")
      } else {
        try await eventually {
          (try? model.panes.layout().tabs.last?.agentSessionID) == "agent-session"
        }
        XCTAssertEqual(try model.panes.layout().tabs.last?.kind, .session)
        XCTAssertEqual(model.panes.controller.allTabIds.count, 1)
        if method == "RestoreAgentSession" {
          let request = try Releash_Client_V1_RestoreAgentSessionRequest(
            serializedBytes: XCTUnwrap(http.requests(method).last?.message))
          XCTAssertEqual(request.agentSessionID, "archived")
        } else {
          let request = try Releash_Client_V1_ResumeAgentSessionHistoryCandidateRequest(
            serializedBytes: XCTUnwrap(http.requests(method).last?.message))
          XCTAssertEqual(request.provider, "codex")
          XCTAssertEqual(request.providerSessionID, "history-provider")
          XCTAssertEqual(request.worktreePath, "/repo")
        }
      }
      XCTAssertNotNil(host.element("Claude · archived-provider"))
      tree.snapshot.archivedSessions.items = []
      if method != "DeleteAgentSession" {
        var root = Releash_Client_V1_WorkspaceTreeItem()
        root.node.id = "session-node"
        root.node.sessionID = "agent-session"
        root.node.title = "Restored session"
        root.node.contentKind.value = .session
        tree.snapshot.nodes.items = [root]
      }
      repo.worktrees.items = [tree]
      payload = .init()
      payload.workspaces.repositories.items = [repo]
      try http.publish(payload, target: "workspaces")
      try await eventuallyView(host) { host.element("Claude · archived-provider") == nil }
      if method != "DeleteAgentSession" {
        var snapshot = Releash_Client_V1_SessionTabSnapshot()
        snapshot.session.id = "agent-session"
        snapshot.session.workspaceWorktreePath = "/repo"
        snapshot.session.availability = .paused
        snapshot.node.title = "Restored session"
        http.state.withLock { $0.sessionTabs["agent-session"] = snapshot }
        let roots = await ViewHarness(
          VStack {
            RootListView(model: model, path: "/repo")
            WorkspacePaneView(model: model)
          })
        try await eventually { roots.element("root:session-node") != nil }
        try await eventually {
          model.panes.controller.allPaneIds.flatMap { model.panes.controller.tabs(inPane: $0) }
            .last?.title == "Restored session"
        }
        roots.close()
      }
      host.close()
      await model.stop()
    }
  }
}

extension SessionHistoryViewTests {
  @MainActor func testWorkflowHistoryRestoresExecutionAndReportsFailure() async throws {
    let http = TestHTTPClient()
    var history = Releash_Client_V1_WorkspaceWorkflowHistoryItem()
    history.executionID = "archived-flow"
    history.title = "Archived workflow"
    var tree = Releash_Client_V1_WorkspaceWorktreeList()
    tree.path = "/repo"
    tree.workflowHistory.items = [history]
    var repo = Releash_Client_V1_WorkspaceRepositoryList()
    repo.worktrees.items = [tree]
    http.state.withLock { $0.repositories = [repo] }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    let host = await ViewHarness(SessionHistoryView(model: model, path: "/repo"))
    try await eventuallyView(host) { host.element("Archived workflow") != nil }
    http.state.withLock { $0.failedMethod = "RestoreWorkspaceWorkflowExecution" }
    try await host.press("Archived workflow")
    try await eventuallyView(host) {
      host.texts().contains("RestoreWorkspaceWorkflowExecution failed")
    }
    http.state.withLock { $0.failedMethod = nil }
    try await host.press("Archived workflow")
    try await eventually { http.requests("RestoreWorkspaceWorkflowExecution").count == 2 }
    let request = try Releash_Client_V1_RestoreWorkspaceWorkflowExecutionRequest(
      serializedBytes: XCTUnwrap(http.requests("RestoreWorkspaceWorkflowExecution").last?.message))
    XCTAssertEqual(request.worktreePath, "/repo")
    XCTAssertEqual(request.executionID, "archived-flow")
    var root = Releash_Client_V1_WorkspaceTreeItem()
    root.sequence.id = "archived-flow"
    root.sequence.title = "Archived workflow"
    tree.workflowHistory.items = []
    tree.snapshot.nodes.items = [root]
    repo.worktrees.items = [tree]
    var payload = Releash_Client_V1_StatePayload()
    payload.workspaces.repositories.items = [repo]
    try http.publish(payload, target: "workspaces")
    let nodes = await ViewHarness(RootListView(model: model, path: "/repo"))
    try await eventuallyView(nodes) { nodes.element("root:archived-flow") != nil }
    nodes.close()
    host.close()
    await model.stop()
  }
}

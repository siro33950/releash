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
      if method == "RestoreAgentSession" {
        model.panes.openNode(.command, id: "command", title: "Command")
      } else if method == "ResumeAgentSessionHistoryCandidate" {
        model.panes.openPendingSession(id: "session-tree", title: "Preparing")
      }
      let originalTabs = model.panes.controller.allTabIds
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
        // サーバがタブを足す（Restore）か、仮タブに Session を結び付ける（Resume）配置を配信する
        guard case .pane(let id, var tabs, let active) = try model.panes.layout() else {
          return XCTFail("Expected a single pane")
        }
        if method == "RestoreAgentSession" {
          tabs.append(PaneTab(kind: .session, agentSessionID: "agent-session"))
        } else {
          let pending = try XCTUnwrap(
            tabs.firstIndex { $0.kind == .session && $0.nodeID == "session-tree" })
          tabs[pending].nodeID = nil
          tabs[pending].agentSessionID = "agent-session"
        }
        let layout = PaneLayout.pane(id: id, tabs: tabs, active: active)
        var state = Releash_Client_V1_StatePayload()
        state.workspaceState.value.version = 1
        state.workspaceState.value.paneLayout = layout.message
        try http.publish(state, target: "workspace-state")
        try await eventually { (try? model.panes.layout()) == layout }
        if method == "RestoreAgentSession" {
          let pane = try XCTUnwrap(model.panes.controller.allPaneIds.first)
          XCTAssertEqual(model.panes.controller.selectedTab(inPane: pane)?.id, originalTabs.first)
        }
        try await eventually {
          (try? model.panes.layout().tabs.last?.agentSessionID) == "agent-session"
        }
        XCTAssertEqual(try model.panes.layout().tabs.last?.kind, .session)
        if method == "RestoreAgentSession" {
          let request = try Releash_Client_V1_RestoreAgentSessionRequest(
            serializedBytes: XCTUnwrap(http.requests(method).last?.message))
          XCTAssertEqual(request.agentSessionID, "archived")
        } else {
          XCTAssertEqual(model.panes.controller.allTabIds, originalTabs)
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
        root.node.id = "session-tree"
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
        try await eventually { roots.element("root:session-tree") != nil }
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
  @MainActor func testSessionRestoreDismissesSheetOnlyOnSuccessWithoutChangingSelection()
    async throws
  {
    let http = TestHTTPClient()
    var session = Releash_Client_V1_AgentSessionItemDto()
    session.id = "archived"
    session.provider.value = .claude
    session.providerSessionID = "archived-provider"
    var tree = Releash_Client_V1_WorkspaceWorktreeList()
    tree.path = "/repo"
    tree.snapshot.archivedSessions.items = [session]
    var repo = Releash_Client_V1_WorkspaceRepositoryList()
    repo.worktrees.items = [tree]
    http.state.withLock { $0.repositories = [repo] }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    model.panes.openSession(id: "agent-session", nodeID: "session-tree", title: "Session")
    model.panes.openNode(.command, id: "command", title: "Command")
    let layout = model.panes.layoutInput
    let host = await ViewHarness(HistoryRestoreSheet(model: model))
    defer { host.close() }
    try await host.press("Show history")
    try await eventually { !host.window.sheets.isEmpty }
    try await eventuallyView(host) { host.element("Claude · archived-provider") != nil }
    http.state.withLock { $0.failedMethod = "RestoreAgentSession" }
    try await host.press("Claude · archived-provider")
    try await eventuallyView(host) { host.texts().contains("RestoreAgentSession failed") }
    XCTAssertFalse(host.window.sheets.isEmpty)
    XCTAssertEqual(model.panes.layoutInput, layout)

    http.state.withLock { $0.failedMethod = nil }
    try await host.press("Claude · archived-provider")
    try await eventually { host.window.sheets.isEmpty }
    XCTAssertEqual(http.requests("RestoreAgentSession").count, 2)
    XCTAssertEqual(model.panes.layoutInput, layout)
    await model.stop()
  }

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

private struct HistoryRestoreSheet: View {
  let model: AppModel
  @State private var showing = false

  var body: some View {
    Button("Show history") { showing = true }
      .sheet(isPresented: $showing) { SessionHistoryView(model: model, path: "/repo") }
  }
}

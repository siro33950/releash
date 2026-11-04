import Bonsplit
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class SessionTabTests: XCTestCase {
  @MainActor func test_会話タブ_復元後も同じ会話を選択し名前と全状態色を更新する() throws {
    // Given
    let workspace = PaneWorkspace()
    try workspace.restore(.empty())
    // When
    workspace.openNode(.session, id: "agent", title: "Session")
    let tab = try XCTUnwrap(workspace.controller.allTabIds.first)
    workspace.openNode(.command, id: "agent", title: "Command")
    workspace.openNode(.session, id: "agent", title: "Session")
    // Then
    XCTAssertEqual(workspace.controller.allTabIds.count, 2)
    XCTAssertEqual(
      workspace.controller.selectedTab(inPane: workspace.controller.allPaneIds[0])?.id, tab)
    // Given
    var snapshot = Releash_Client_V1_SessionTabSnapshot()
    snapshot.session.id = "agent"
    snapshot.node.title = "Renamed Session"
    for state in [Releash_Client_V1_WorkspaceStatusClassification.Value.attention, .active, .idle] {
      // Given
      snapshot.node.classification.value = state
      let placement = workspace.layoutInput
      // When
      workspace.updateSession(tab, snapshot: snapshot)
      // Then
      XCTAssertEqual(workspace.layoutInput, placement)
      XCTAssertEqual(workspace.controller.tab(tab)?.title, "Renamed Session")
      let color = try XCTUnwrap(workspace.controller.tab(tab)?.iconColor)
      let actual = try XCTUnwrap(NSColor(color).usingColorSpace(.sRGB))
      let expected = try XCTUnwrap(
        NSColor(UIStyle.statusColor(String(describing: state))).usingColorSpace(.sRGB))
      XCTAssertEqual(actual.redComponent, expected.redComponent, accuracy: 0.000001)
      XCTAssertEqual(actual.greenComponent, expected.greenComponent, accuracy: 0.000001)
      XCTAssertEqual(actual.blueComponent, expected.blueComponent, accuracy: 0.000001)
      XCTAssertEqual(actual.alphaComponent, expected.alphaComponent, accuracy: 0.000001)
      XCTAssertEqual(workspace.controller.tab(tab)?.icon, "sparkle")
    }
    // Given
    let restored = PaneWorkspace()
    // When
    try restored.restore(PaneLayout(workspace.layout().message))
    restored.openNode(.session, id: "agent", title: "Session")
    // Then
    XCTAssertEqual(restored.controller.allTabIds.count, 2)
    XCTAssertEqual(
      try restored.layout().tabs.filter { $0.kind == .session }.map(\.agentSessionID), ["agent"])
    XCTAssertNil(try restored.layout().tabs.first?.nodeID)
  }

  @MainActor func test_会話タブ_新規作成と一覧で同じ会話を選択する() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    var root = Releash_Client_V1_WorkspaceTreeItem()
    root.node.id = "session-tree"
    root.node.sessionID = "agent-session"
    root.node.contentKind.value = .session
    // Given
    model.openRoot(
      {
        var pending = root
        pending.node.clearSessionID()
        return pending
      }())
    let original = try XCTUnwrap(model.panes.controller.allTabIds.first)
    // When: 起動の応答は仮タブに触れない
    try await model.startSession(provider: "codex", path: "/repo")
    // Then
    XCTAssertEqual(model.panes.controller.allTabIds, [original])
    XCTAssertEqual(try model.panes.layout().tabs.first?.nodeID, "session-tree")
    XCTAssertNil(try model.panes.layout().tabs.first?.agentSessionID)
    // When: サーバが仮タブに Session を結び付けた配置を配信し、一覧からも同じ会話を開く
    try await eventually { model.pendingSaves["/repo"] == nil }
    guard case .pane(let paneID, _, _) = try model.panes.layout() else {
      return XCTFail("Expected a single pane")
    }
    var payload = Releash_Client_V1_StatePayload()
    payload.workspaceState.value.paneLayout =
      PaneLayout.pane(
        id: paneID,
        tabs: [
          PaneTab(id: original.id.uuidString, kind: .session, agentSessionID: "agent-session")
        ],
        active: original.id.uuidString
      ).message
    payload.workspaceState.value.etag = "9"
    http.state.withLock { $0.layouts["/repo"] = payload.workspaceState.value }
    try http.publish(payload, target: "workspace-state")
    try await eventually {
      (try? model.panes.layout().tabs.first?.agentSessionID) == "agent-session"
    }
    model.openRoot(root)
    // Then
    XCTAssertEqual(model.panes.controller.allTabIds, [original])
    XCTAssertEqual(try model.panes.layout().tabs.first?.agentSessionID, "agent-session")
    XCTAssertNil(try model.panes.layout().tabs.first?.nodeID)
    await model.stop()
  }

  @MainActor func test_会話の新規作成_画面はタブを作らず配信で届いたタブを応答の前後どちらでも選ぶ() async throws {
    for pushFirst in [false, true] {
      // Given: 別の会話のタブを 1 つ選択している
      let http = TestHTTPClient()
      let model = try await testModel(http)
      model.selectWorktree("/repo")
      try await eventually { model.layoutLoaded }
      let other = PaneTab(id: "other", kind: .session, agentSessionID: "other")
      let added = PaneTab(
        id: "session-agent-session", kind: .session, agentSessionID: "agent-session")
      var payload = Releash_Client_V1_StatePayload()
      payload.workspaceState.value.paneLayout =
        PaneLayout.pane(id: "pane", tabs: [other], active: other.id).message
      payload.workspaceState.value.etag = "1"
      http.state.withLock { $0.layouts["/repo"] = payload.workspaceState.value }
      try http.publish(payload, target: "workspace-state")
      try await eventually { model.panes.controller.allTabIds.count == 1 }
      func selectedSession() -> String? {
        guard let pane = model.panes.controller.allPaneIds.first,
          let tab = model.panes.controller.selectedTab(inPane: pane)?.id
        else { return nil }
        return model.panes.tab(tab)?.agentSessionID
      }
      func pushAdded() throws {
        payload.workspaceState.value.paneLayout =
          PaneLayout.pane(id: "pane", tabs: [other, added], active: other.id).message
        payload.workspaceState.value.etag = "2"
        http.state.withLock { $0.layouts["/repo"] = payload.workspaceState.value }
        try http.publish(payload, target: "workspace-state")
      }
      if pushFirst {
        try pushAdded()
        try await eventually { model.panes.controller.allTabIds.count == 2 }
      }
      let saves = http.requests("SaveWorkspaceState").count
      // When: 起動の応答が届く
      try await model.startSession(provider: "codex", path: "/repo")
      if !pushFirst {
        // Then: 画面はタブを作らず、保存もせず、選択も変えない
        XCTAssertEqual(model.panes.controller.allTabIds.count, 1)
        XCTAssertEqual(http.requests("SaveWorkspaceState").count, saves)
        XCTAssertEqual(selectedSession(), "other")
        // When: サーバが末尾にタブを足した配置を配信する
        try pushAdded()
      }
      // Then: 足されたタブが選ばれ、その選択が保存される
      try await eventually { model.panes.controller.allTabIds.count == 2 }
      try await eventually { selectedSession() == "agent-session" }
      try await eventually {
        guard let data = http.requests("SaveWorkspaceState").last?.message,
          let saved = try? Releash_Client_V1_SaveWorkspaceStateRequest(serializedBytes: data),
          let layout = try? PaneLayout(saved.state.paneLayout),
          case .pane(_, _, let active) = layout
        else { return false }
        return active == added.id
      }
      XCTAssertEqual(try model.panes.layout().tabs.map(\.id), [other.id, added.id])
      await model.stop()
    }
  }

  @MainActor func test_会話の新規作成_待っている会話より後に開いた会話を選んだら待ちを捨てる() async throws {
    // Given: 会話 "agent-session" の応答を受けたが、そのタブはまだ届いていない
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    let other = PaneTab(id: "session-other", kind: .session, agentSessionID: "other")
    let added = PaneTab(
      id: "session-agent-session", kind: .session, agentSessionID: "agent-session")
    var payload = Releash_Client_V1_StatePayload()
    payload.workspaceState.value.paneLayout =
      PaneLayout.pane(id: "pane", tabs: [other], active: nil).message
    payload.workspaceState.value.etag = "1"
    http.state.withLock { $0.layouts["/repo"] = payload.workspaceState.value }
    try http.publish(payload, target: "workspace-state")
    try await eventually { model.panes.controller.allTabIds.count == 1 }
    try await model.startSession(provider: "codex", path: "/repo")
    // When: 既に届いている別の会話 "other" を開き、その後に "agent-session" のタブが届く
    var later = Releash_Client_V1_SessionSelection()
    later.agentSessionID = "other"
    model.openSession(later, path: "/repo")
    try await eventually { model.pendingSaves["/repo"] == nil }
    payload.workspaceState.value.paneLayout =
      PaneLayout.pane(id: "pane", tabs: [other, added], active: other.id).message
    payload.workspaceState.value.etag = http.state.withLock { $0.layouts["/repo"]?.etag ?? "2" }
    http.state.withLock { $0.layouts["/repo"] = payload.workspaceState.value }
    try http.publish(payload, target: "workspace-state")
    try await eventually { model.panes.controller.allTabIds.count == 2 }
    // Then: 後に開いた "other" の選択が保たれる
    let pane = try XCTUnwrap(model.panes.controller.allPaneIds.first)
    let selected = try XCTUnwrap(model.panes.controller.selectedTab(inPane: pane)?.id)
    XCTAssertEqual(model.panes.tab(selected)?.agentSessionID, "other")
    await model.stop()
  }

  @MainActor func test_会話タブ_準備中タブなしで会話を開いた後も古い一覧から同じタブを選ぶ() throws {
    for alreadyOpen in [false, true] {
      // Given
      let workspace = PaneWorkspace()
      try workspace.restore(.empty())
      let pane = try XCTUnwrap(workspace.controller.allPaneIds.first)
      let existing = alreadyOpen ? workspace.openNode(.session, id: "agent", title: "Ready") : nil
      let command = try XCTUnwrap(workspace.openNode(.command, id: "agent", title: "Command"))
      // When
      workspace.openSession(id: "agent", nodeID: "node", title: "Ready")
      // Then
      let session = try XCTUnwrap(workspace.controller.selectedTab(inPane: pane)?.id)
      XCTAssertNotEqual(session, command)
      if alreadyOpen { XCTAssertEqual(session, existing) }
      // Given
      workspace.controller.selectTab(command)
      // When
      workspace.openPendingSession(id: "node", title: "Preparing")
      // Then
      XCTAssertEqual(workspace.controller.allTabIds.count, 2)
      XCTAssertEqual(workspace.controller.selectedTab(inPane: pane)?.id, session)
      let saved = try PaneLayout(workspace.layout().message).tabs.filter { $0.kind == .session }
      XCTAssertEqual(saved.map(\.id), [session.id.uuidString])
      XCTAssertEqual(saved.map(\.agentSessionID), ["agent"])
      XCTAssertNil(saved.first?.nodeID)
    }
  }

  @MainActor func test_会話タブ_詳細の紐付け後に古い一覧を繰り返し開いても同じタブを選ぶ() throws {
    // Given
    let workspace = PaneWorkspace()
    try workspace.restore(.empty())
    workspace.openPendingSession(id: "node", title: "Preparing")
    let tab = try XCTUnwrap(workspace.controller.allTabIds.first)
    // When
    XCTAssertEqual(workspace.bindSession(tab, sessionID: "agent"), tab)
    for _ in 0..<3 { workspace.openPendingSession(id: "node", title: "Preparing") }
    // Then
    XCTAssertEqual(workspace.controller.allTabIds, [tab])
    XCTAssertEqual(
      workspace.controller.selectedTab(inPane: workspace.controller.allPaneIds[0])?.id, tab)
    XCTAssertEqual(try workspace.layout().tabs.first?.agentSessionID, "agent")
    XCTAssertNil(try workspace.layout().tabs.first?.nodeID)
    // When
    workspace.controller.closeTab(tab)
    workspace.openPendingSession(id: "node", title: "Preparing")
    // Then
    let reopened = try XCTUnwrap(workspace.controller.allTabIds.first)
    XCTAssertNotEqual(reopened, tab)
    XCTAssertNil(workspace.bindSession(tab, sessionID: "agent"))
    XCTAssertNil(workspace.tab(reopened)?.agentSessionID)
  }

  @MainActor func test_会話タブ_別ペインの既存会話へ準備中タブの解決を集約する() throws {
    for detailFirst in [false, true] {
      // Given
      let workspace = PaneWorkspace()
      try workspace.restore(.empty())
      workspace.openNode(.session, id: "agent", title: "Ready")
      let existing = try XCTUnwrap(workspace.controller.allTabIds.first)
      let first = try XCTUnwrap(workspace.controller.allPaneIds.first)
      let second = try XCTUnwrap(workspace.controller.splitPane(first, orientation: .vertical))
      workspace.controller.focusPane(second)
      workspace.openPendingSession(id: "node", title: "Preparing")
      let pending = try XCTUnwrap(workspace.controller.allTabIds.first { $0 != existing })
      // When
      if detailFirst {
        XCTAssertEqual(workspace.bindSession(pending, sessionID: "agent"), existing)
      } else {
        workspace.openSession(id: "agent", nodeID: "node", title: "Ready")
      }
      workspace.openPendingSession(id: "node", title: "Preparing")
      // Then
      XCTAssertEqual(workspace.controller.allTabIds, [existing])
      XCTAssertEqual(workspace.controller.focusedPaneId, first)
      XCTAssertEqual(workspace.controller.selectedTab(inPane: first)?.id, existing)
      XCTAssertNil(workspace.bindSession(pending, sessionID: "agent"))
      XCTAssertEqual(try workspace.layout().tabs.map(\.agentSessionID), ["agent"])
    }
  }

  @MainActor func test_配置保存の監視_分割率と選択と参照の変更を検知する() throws {
    // Given
    let workspace = PaneWorkspace()
    let first = UUID().uuidString
    let second = UUID().uuidString
    let split = UUID().uuidString
    let left = PaneLayout.pane(
      id: UUID().uuidString,
      tabs: [
        PaneTab(id: first, kind: .session, agentSessionID: "first"),
        PaneTab(id: second, kind: .session, agentSessionID: "second"),
      ], active: first)
    let right = PaneLayout.empty()
    try workspace.restore(
      .split(id: split, axis: .horizontal, ratio: 0.4, first: left, second: right))
    let original = workspace.layoutInput
    // Then
    XCTAssertEqual(workspace.layoutInput, original)
    // When
    try workspace.restore(
      .split(id: split, axis: .horizontal, ratio: 0.6, first: left, second: right))
    // Then
    XCTAssertNotEqual(workspace.layoutInput, original)
    let resized = workspace.layoutInput
    // When
    workspace.controller.selectTab(TabID(id: try XCTUnwrap(UUID(uuidString: second))))
    // Then
    XCTAssertNotEqual(workspace.layoutInput, resized)
    let selected = workspace.layoutInput
    // When
    workspace.bindSession(TabID(id: try XCTUnwrap(UUID(uuidString: second))), sessionID: "bound")
    // Then
    XCTAssertNotEqual(workspace.layoutInput, selected)
  }

  @MainActor func test_会話タブ操作_改名と確認付き整理と取消と操作制限を反映する() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    var snapshot = Releash_Client_V1_SessionTabSnapshot()
    snapshot.session.workspaceWorktreePath = "/repo"
    snapshot.session.treeLocation.treeID = "tree"
    snapshot.session.operations.canArchive = true
    snapshot.node.id = "node"
    snapshot.node.canAbort = true
    // When
    model.requestArchiveSessionTab(snapshot)
    // Then
    XCTAssertEqual(model.panes.archivingSession, snapshot)
    XCTAssertTrue(http.requests("ArchiveWorkspaceWorkflowExecution").isEmpty)
    // When
    model.panes.archivingSession = nil
    model.confirmArchiveSessionTab()
    // Then
    XCTAssertTrue(http.requests("ArchiveWorkspaceWorkflowExecution").isEmpty)
    // When
    model.requestArchiveSessionTab(snapshot)
    model.confirmArchiveSessionTab()
    // Then
    try await eventually { http.requests("ArchiveWorkspaceWorkflowExecution").count == 1 }
    XCTAssertNil(model.panes.archivingSession)
    // Given
    snapshot.node.canAbort = false
    // When
    model.requestArchiveSessionTab(snapshot)
    // Then
    try await eventually { http.requests("ArchiveWorkspaceWorkflowExecution").count == 2 }
    // Given
    snapshot.session.operations.canArchive = false
    snapshot.node.canAbort = true
    // When
    model.requestArchiveSessionTab(snapshot)
    // Then
    XCTAssertNil(model.panes.archivingSession)
    XCTAssertEqual(http.requests("ArchiveWorkspaceWorkflowExecution").count, 2)
    // When
    model.renameSessionTab(snapshot, name: "Renamed")
    // Then
    try await eventually { http.requests("RenameWorkspaceSessionNode").count == 1 }
    let rename = try Releash_Client_V1_RenameWorkspaceSessionNodeRequest(
      serializedBytes: XCTUnwrap(http.requests("RenameWorkspaceSessionNode").first?.message))
    XCTAssertEqual(rename.name, "Renamed")
    XCTAssertEqual(rename.nodeID, "node")
    XCTAssertEqual(rename.worktreePath, "/repo")
    let archive = try Releash_Client_V1_ArchiveWorkspaceWorkflowExecutionRequest(
      serializedBytes: XCTUnwrap(http.requests("ArchiveWorkspaceWorkflowExecution").first?.message))
    XCTAssertEqual(archive.executionID, "tree")
    XCTAssertEqual(archive.worktreePath, "/repo")
    // Given
    snapshot.node.canAbort = false
    snapshot.session.operations.canArchive = true
    http.state.withLock { $0.failedMethod = "ArchiveWorkspaceWorkflowExecution" }
    // When
    model.requestArchiveSessionTab(snapshot)
    // Then
    try await eventually { model.failure == "ArchiveWorkspaceWorkflowExecution failed" }
    // Given
    http.state.withLock { $0.failedMethod = "RenameWorkspaceSessionNode" }
    // When
    model.renameSessionTab(snapshot, name: "failure")
    // Then
    try await eventually { model.failure == "RenameWorkspaceSessionNode failed" }
    await model.stop()
  }
}

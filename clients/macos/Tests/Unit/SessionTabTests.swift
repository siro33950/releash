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
      // When
      workspace.updateSession(tab, snapshot: snapshot)
      // Then
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
    root.node.id = "tree"
    root.node.sessionID = "agent-session"
    root.node.contentKind.value = .session
    // When
    try await model.startSession(provider: "codex", path: "/repo")
    model.openRoot(root)
    // Then
    XCTAssertEqual(model.panes.controller.allTabIds.count, 1)
    XCTAssertEqual(try model.panes.layout().tabs.first?.agentSessionID, "agent-session")
    await model.stop()
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

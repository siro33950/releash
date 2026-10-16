import Bonsplit
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class SessionTabTests: XCTestCase {
  @MainActor func testSessionIdentitySurvivesRestoreAndUpdatesTitleAndEveryStateColor() throws {
    let workspace = PaneWorkspace()
    try workspace.restore(.empty())
    workspace.openNode(.session, id: "agent", title: "Session")
    let tab = try XCTUnwrap(workspace.controller.allTabIds.first)
    workspace.openNode(.command, id: "agent", title: "Command")
    workspace.openNode(.session, id: "agent", title: "Session")
    XCTAssertEqual(workspace.controller.allTabIds.count, 2)
    XCTAssertEqual(
      workspace.controller.selectedTab(inPane: workspace.controller.allPaneIds[0])?.id, tab)
    var snapshot = Releash_Client_V1_SessionTabSnapshot()
    snapshot.session.id = "agent"
    snapshot.node.title = "Renamed Session"
    for state in [Releash_Client_V1_WorkspaceStatusClassification.Value.attention, .active, .idle] {
      snapshot.node.classification.value = state
      workspace.updateSession(tab, snapshot: snapshot)
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
    let restored = PaneWorkspace()
    try restored.restore(PaneLayout(workspace.layout().message))
    restored.openNode(.session, id: "agent", title: "Session")
    XCTAssertEqual(restored.controller.allTabIds.count, 2)
    XCTAssertEqual(
      try restored.layout().tabs.filter { $0.kind == .session }.map(\.agentSessionID), ["agent"])
    XCTAssertNil(try restored.layout().tabs.first?.nodeID)
  }

  @MainActor func testNewSessionAndListSelectSameSessionWithDifferentNodeAndTreeIDs() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    try await model.startSession(provider: "codex", path: "/repo")
    var root = Releash_Client_V1_WorkspaceTreeItem()
    root.node.id = "tree"
    root.node.sessionID = "agent-session"
    root.node.contentKind.value = .session
    model.openRoot(root)
    XCTAssertEqual(model.panes.controller.allTabIds.count, 1)
    XCTAssertEqual(try model.panes.layout().tabs.first?.agentSessionID, "agent-session")
    await model.stop()
  }

  @MainActor func testRenameArchiveConfirmationCancellationAndWorkflowRestriction() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    var snapshot = Releash_Client_V1_SessionTabSnapshot()
    snapshot.session.workspaceWorktreePath = "/repo"
    snapshot.session.treeLocation.treeID = "tree"
    snapshot.session.operations.canArchive = true
    snapshot.node.id = "node"
    snapshot.node.canAbort = true
    model.requestArchiveSessionTab(snapshot)
    XCTAssertEqual(model.panes.archivingSession, snapshot)
    XCTAssertTrue(http.requests("ArchiveWorkspaceWorkflowExecution").isEmpty)
    model.panes.archivingSession = nil
    model.confirmArchiveSessionTab()
    XCTAssertTrue(http.requests("ArchiveWorkspaceWorkflowExecution").isEmpty)
    model.requestArchiveSessionTab(snapshot)
    model.confirmArchiveSessionTab()
    try await eventually { http.requests("ArchiveWorkspaceWorkflowExecution").count == 1 }
    XCTAssertNil(model.panes.archivingSession)
    snapshot.node.canAbort = false
    model.requestArchiveSessionTab(snapshot)
    try await eventually { http.requests("ArchiveWorkspaceWorkflowExecution").count == 2 }
    snapshot.session.operations.canArchive = false
    snapshot.node.canAbort = true
    model.requestArchiveSessionTab(snapshot)
    XCTAssertNil(model.panes.archivingSession)
    XCTAssertEqual(http.requests("ArchiveWorkspaceWorkflowExecution").count, 2)
    model.renameSessionTab(snapshot, name: "Renamed")
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
    snapshot.node.canAbort = false
    snapshot.session.operations.canArchive = true
    http.state.withLock { $0.failedMethod = "ArchiveWorkspaceWorkflowExecution" }
    model.requestArchiveSessionTab(snapshot)
    try await eventually { model.failure == "ArchiveWorkspaceWorkflowExecution failed" }
    http.state.withLock { $0.failedMethod = "RenameWorkspaceSessionNode" }
    model.renameSessionTab(snapshot, name: "failure")
    try await eventually { model.failure == "RenameWorkspaceSessionNode failed" }
    await model.stop()
  }
}

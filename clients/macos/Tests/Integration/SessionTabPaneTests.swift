import Bonsplit
import SwiftUI
import XCTest

@testable import Releash

final class SessionTabPaneTests: XCTestCase {
  @MainActor func testIDOnlySubscriptionUpdatesTitleAvailabilityAndReleasesTerminal() async throws {
    let http = TestHTTPClient()
    var snapshot = Releash_Client_V1_SessionTabSnapshot()
    snapshot.session.id = "agent"
    snapshot.session.workspaceWorktreePath = "/repo"
    snapshot.session.availability = .open
    snapshot.node.id = "node"
    snapshot.node.title = "Before"
    snapshot.node.classification.value = .active
    http.state.withLock { $0.sessionTabs["agent"] = snapshot }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    model.openNode(.session, id: "agent", title: "Session")
    let tab = try XCTUnwrap(model.panes.controller.allTabIds.first)
    let host = await ViewHarness(
      AnyView(SessionTabPane(model: model, tabID: tab, sessionID: "agent", focused: true)))
    defer { host.close() }
    try await eventuallyView(host) { host.element("terminal:agent") != nil }
    XCTAssertTrue(
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "session-tab" && $0.args == ["agent"] }
      })
    XCTAssertFalse(
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "node-detail" || $0.target == "agent-session" }
      })
    for (availability, message) in [
      (Releash_Client_V1_AgentSessionAvailability.indeterminate, "Sessionを開けるか判定できません"),
      (.archived, "SessionはArchiveされています"),
      (.paused, "Sessionは停止しています"),
    ] {
      snapshot.session.availability = availability
      snapshot.node.title = "After"
      snapshot.node.classification.value = .idle
      var payload = Releash_Client_V1_StatePayload()
      payload.sessionTab = snapshot
      try http.publish(payload, target: "session-tab")
      try await eventuallyView(host) { host.texts().contains(message) }
      XCTAssertNil(host.element("terminal:agent"))
      XCTAssertEqual(model.panes.controller.tab(tab)?.title, "After")
      try await eventually {
        model.terminalAttachment(path: "/repo", id: "agent", agent: true) == nil
      }
    }
    snapshot.node.capabilities.canResumeSession = true
    var payload = Releash_Client_V1_StatePayload()
    payload.sessionTab = snapshot
    try http.publish(payload, target: "session-tab")
    try await eventuallyView(host) { host.element("Resume") != nil }
    try await host.press("Resume")
    try await eventually { http.requests("ResumeWorkspaceSessionNode").count == 1 }
    await model.stop()
  }
}

extension SessionTabPaneTests {
  @MainActor func testTabMenuRenamesAndConfirmsOrCancelsArchiveAndHidesWorkflowArchive()
    async throws
  {
    let http = TestHTTPClient()
    var snapshot = Releash_Client_V1_SessionTabSnapshot()
    snapshot.session.id = "agent"
    snapshot.session.workspaceWorktreePath = "/repo"
    snapshot.session.treeLocation.treeID = "tree"
    snapshot.session.availability = .paused
    snapshot.session.operations.canArchive = true
    snapshot.node.id = "node"
    snapshot.node.title = "Session title"
    snapshot.node.canAbort = true
    snapshot.node.capabilities.canRename = true
    http.state.withLock { $0.sessionTabs["agent"] = snapshot }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    model.openNode(.session, id: "agent", title: "Session")
    let host = await ViewHarness(WorkspacePaneView(model: model))
    defer { host.close() }
    try await eventuallyView(host) { host.element("Session title") != nil }
    try await host.menu("Session title", selecting: "Rename") {
      XCTAssertTrue($0.contains("Rename"))
    }
    try await eventuallyView(host) { host.element("保存") != nil }
    try await host.setTextField("Renamed")
    try await host.pressAlertButton("保存")
    try await eventually { http.requests("RenameWorkspaceSessionNode").count == 1 }
    try await host.menu("Session title", selecting: "Archive") {
      XCTAssertTrue($0.contains("Archive"))
    }
    try await eventuallyView(host) { host.element("Abort and Archive") != nil }
    try await host.pressAlertButton("キャンセル")
    XCTAssertTrue(http.requests("ArchiveWorkspaceWorkflowExecution").isEmpty)
    try await host.menu("Session title", selecting: "Archive") {
      XCTAssertTrue($0.contains("Archive"))
    }
    try await eventuallyView(host) { host.element("Abort and Archive") != nil }
    try await host.pressAlertButton("Abort and Archive")
    try await eventually { http.requests("ArchiveWorkspaceWorkflowExecution").count == 1 }
    snapshot.node.canAbort = false
    var payload = Releash_Client_V1_StatePayload()
    payload.sessionTab = snapshot
    try http.publish(payload, target: "session-tab")
    try await eventually { model.panes.sessionSnapshots.values.first?.node.canAbort == false }
    try await host.menu("Session title", selecting: "Archive") {
      XCTAssertTrue($0.contains("Archive"))
    }
    try await eventually { http.requests("ArchiveWorkspaceWorkflowExecution").count == 2 }
    snapshot.session.operations.canArchive = false
    payload.sessionTab = snapshot
    try http.publish(payload, target: "session-tab")
    try await eventually {
      model.panes.sessionSnapshots.values.first?.session.operations.canArchive == false
    }
    try await host.menu("Session title", selecting: nil) {
      XCTAssertTrue($0.contains("Rename"))
      XCTAssertFalse($0.contains("Archive"))
    }
    await model.stop()
  }
}

import AppKit
import SwiftProtobuf
import XCTest

@testable import Releash

final class SessionHistoryFailureTests: XCTestCase {
  @MainActor func testSessionRestoreFailureKeepsSheetAndTabsAndCanRetry() async throws {
    try await checkFailure(method: "RestoreAgentSession", row: "Archived session", action: "復元")
  }

  @MainActor func testSessionDeleteFailureKeepsSheetAndTabsAndCanRetry() async throws {
    try await checkFailure(method: "DeleteAgentSession", row: "Archived session", action: "削除")
  }

  @MainActor func testProviderResumeFailureKeepsSheetAndTabsAndCanRetry() async throws {
    try await checkFailure(
      method: "ResumeAgentSessionHistoryCandidate", row: "Provider session", action: "再開")
  }

  @MainActor func testWorkflowRestoreFailureKeepsSheetAndTabsAndCanRetry() async throws {
    try await checkFailure(
      method: "RestoreWorkspaceWorkflowExecution", row: "Archived workflow", action: "復元")
  }

  @MainActor private func checkFailure(method: String, row: String, action: String) async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.history.items.items = [
        .with {
          $0.provider.value = .claude
          $0.providerSessionID = "provider"
          $0.label = "Provider session"
        }
      ]
    }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    model.panes.openSession(id: "existing", nodeID: "existing-node", title: "Existing session")
    model.panes.openNode(.command, id: "command", title: "Command")
    let layout = model.panes.layoutInput
    let host = await ViewHarness(RootListView(model: model, path: "/repo"))
    defer { host.close() }
    try await MenuHarness().open(
      selection: ["History…"], press: { try await host.press("More") })
    try await eventually {
      !host.window.sheets.isEmpty
        && http.state.withLock { $0.subscriptions.contains { $0.target == "archived-history" } }
    }
    let sheet = try XCTUnwrap(host.window.sheets.first)
    var payload = Releash_Client_V1_StatePayload()
    payload.archivedHistory.items = [
      .with {
        $0.session.agentSessionID = "archived"
        $0.session.name = "Archived session"
        $0.session.provider.value = .claude
        $0.session.canDelete = true
      },
      .with {
        $0.workflow.executionID = "archived-workflow"
        $0.workflow.worktreePath = "/repo"
        $0.workflow.title = "Archived workflow"
        $0.workflow.status.value = .completed
      },
    ]
    try http.publish(payload, target: "archived-history")
    try await eventuallyView(host) { host.element(row) != nil }
    try await host.press(row)
    http.state.withLock { $0.failedMethod = method }

    // When
    try await perform(action, in: host)

    // Then
    try await eventuallyView(host) { host.texts().contains { $0.contains("\(method) failed") } }
    XCTAssertEqual(http.requests(method).count, 1)
    XCTAssertTrue(host.window.sheets.contains { $0 === sheet })
    XCTAssertEqual(model.panes.layoutInput, layout)
    XCTAssertNotNil(host.element(row))

    // When
    http.state.withLock { $0.failedMethod = nil }
    try await perform(action, in: host)

    // Then
    try await eventually { http.requests(method).count == 2 }
    if method == "DeleteAgentSession" {
      payload.archivedHistory.items.removeFirst()
      try http.publish(payload, target: "archived-history")
      try await eventuallyView(host) {
        host.element(row) == nil && !host.texts().contains { $0.contains("\(method) failed") }
      }
      XCTAssertTrue(host.window.sheets.contains { $0 === sheet })
      XCTAssertEqual(model.panes.layoutInput, layout)
    } else {
      try await eventually { host.window.sheets.isEmpty }
      if method != "ResumeAgentSessionHistoryCandidate" {
        XCTAssertEqual(model.panes.layoutInput, layout)
      }
    }
    await model.stop()
  }

  @MainActor private func perform(_ action: String, in host: ViewHarness) async throws {
    try await host.press(action)
    if action == "削除" {
      try await eventuallyView(host) { host.element("キャンセル") != nil }
      let history = try XCTUnwrap(host.window.sheets.first)
      let alert = try XCTUnwrap(history.attachedSheet)
      func buttons(_ view: NSView) -> [NSButton] {
        (view as? NSButton).map { [$0] } ?? view.subviews.flatMap(buttons)
      }
      let content = try XCTUnwrap(alert.contentView)
      let button = try XCTUnwrap(buttons(content).first { $0.title == "削除" })
      button.performClick(nil)
      try await eventually { history.attachedSheet == nil }
      await host.refresh()
    }
  }
}

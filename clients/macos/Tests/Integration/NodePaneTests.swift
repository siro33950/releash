import AppKit
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class NodePaneTests: XCTestCase {
  @MainActor func testUnsetAvailabilityReleasesOpenTerminal() async throws {
    let http = TestHTTPClient()
    var session = Releash_Client_V1_AgentSessionItemDto()
    session.id = "agent"
    session.availability = .open
    var node = Releash_Client_V1_WorkspaceNodeDetailDto()
    node.id = "node"
    node.content.session.sessionID = "agent"
    http.state.withLock {
      $0.sessions["agent"] = session
      $0.nodeDetails["node"] = node
    }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    let host = await ViewHarness(
      AnyView(NodePane(model: model, path: "/repo", nodeID: "node", focused: true)))
    try await eventuallyView(host) { host.element("terminal:agent") != nil }
    session.clearAvailability()
    var payload = Releash_Client_V1_StatePayload()
    payload.agentSession.value = session
    try http.publish(payload, target: "agent-session")
    try await eventuallyView(host) { host.element("terminal:agent") == nil }
    try await eventually {
      model.terminalAttachment(path: "/repo", id: "agent", agent: true) == nil
    }
    let subscription = try XCTUnwrap(
      http.state.withLock {
        $0.subscriptions.first { $0.target == "node-detail" && $0.args == ["/repo", "node"] }
      })
    let hostingView = try XCTUnwrap(host.window.contentView as? NSHostingView<AnyView>)
    hostingView.rootView = AnyView(EmptyView())
    try await eventually {
      http.requests("StopStateSubscription").contains {
        (try? Releash_Client_V1_StopStateSubscriptionRequest(serializedBytes: $0.message ?? Data())
          .subscriptionID) == subscription.subscriptionID
      }
    }
    host.close()
    await model.stop()
  }

  @MainActor func testSessionSubscriptionControlsVisibilityAndResumeSuccessAndFailure() async throws
  {
    for (availability, canResume, message) in [
      (Releash_Client_V1_AgentSessionAvailability.paused, false, "Sessionは停止しています"),
      (.paused, true, "Sessionは停止しています"), (.indeterminate, true, "Sessionを開けるか判定できません"),
      (.archived, true, "SessionはArchiveされています"),
    ] {
      let http = TestHTTPClient()
      var session = Releash_Client_V1_AgentSessionItemDto()
      session.id = "agent"
      session.availability = availability
      var node = Releash_Client_V1_WorkspaceNodeDetailDto()
      node.id = "node"
      node.title = "Session"
      node.capabilities.canResumeSession = canResume
      node.content.session.sessionID = "agent"
      http.state.withLock {
        $0.sessions["agent"] = session
        $0.nodeDetails["node"] = node
        $0.terminalReplay = "resumed-output"
      }
      let model = try await testModel(http)
      model.selectWorktree("/repo")
      try await eventually { model.layoutLoaded }
      let host = await ViewHarness(
        NodePane(model: model, path: "/repo", nodeID: "node", focused: true))
      try await eventuallyView(host) { host.texts().contains(message) }
      XCTAssertNil(host.element("terminal:agent"))
      XCTAssertEqual(host.element("Resume") != nil, availability == .paused && canResume)
      try await eventually { !http.requests("OpenAgentSession").isEmpty }
      if availability == .paused && canResume {
        http.state.withLock { $0.failedMethod = "ResumeWorkspaceSessionNode" }
        try await host.press("Resume")
        try await eventually { model.failure == "ResumeWorkspaceSessionNode failed" }
        XCTAssertNil(host.element("terminal:agent"))
        XCTAssertTrue(host.texts().contains(message))
        http.state.withLock { $0.failedMethod = nil }
        try await host.press("Resume")
        try await eventually { http.requests("ResumeWorkspaceSessionNode").count == 2 }
        let request = try Releash_Client_V1_ResumeWorkspaceSessionNodeRequest(
          serializedBytes: XCTUnwrap(http.requests("ResumeWorkspaceSessionNode").last?.message))
        XCTAssertEqual(request.nodeID, "node")
        XCTAssertEqual(request.worktreePath, "/repo")
        XCTAssertNil(host.element("terminal:agent"))
        var payload = Releash_Client_V1_StatePayload()
        session.availability = .open
        payload.agentSession.value = session
        try http.publish(payload, target: "agent-session")
        try await eventuallyView(host) { host.element("terminal:agent") != nil }
        try await eventually {
          http.state.withLock {
            $0.subscriptions.contains { $0.target == "terminal" && $0.args == ["/repo", "agent"] }
          }
        }
        try await eventuallyView(host) { host.texts().contains { $0.contains("resumed-output") } }
        XCTAssertEqual(http.requests("OpenAgentSession").count, 1)
        payload.agentSession = .init()
        try http.publish(payload, target: "agent-session")
        try await eventuallyView(host) { host.element("terminal:agent") == nil }
        try await eventually {
          model.terminalAttachment(path: "/repo", id: "agent", agent: true) == nil
        }
      }
      host.close()
      await model.stop()
    }
  }

  @MainActor func testNodeTabsSaveAndRestoreTheirSubscriptionsAndContent() async throws {
    // Given
    let http = TestHTTPClient()
    var session = Releash_Client_V1_AgentSessionItemDto()
    session.id = "agent"
    session.availability = .paused
    session.workspaceWorktreePath = "/repo/a"
    var sessionNode = Releash_Client_V1_WorkspaceNodeDetailDto()
    sessionNode.id = "session-node"
    sessionNode.title = "Restored Session"
    sessionNode.content.session.sessionID = "agent"
    var commandNode = Releash_Client_V1_WorkspaceNodeDetailDto()
    commandNode.id = "command-node"
    commandNode.content.command.result.stdout = "saved command output"
    http.state.withLock {
      var snapshot = Releash_Client_V1_SessionTabSnapshot()
      snapshot.session = session
      snapshot.node = sessionNode
      $0.sessionTabs["agent"] = snapshot
      $0.nodeDetails = ["session-node": sessionNode, "command-node": commandNode]
    }
    let model = try await testModel(http)
    model.selectWorktree("/repo/a")
    try await eventually { model.layoutLoaded }
    let sessionTab = PaneTab(kind: .session, agentSessionID: "agent")
    let commandTab = PaneTab(kind: .command, nodeID: "command-node")
    let layout = PaneLayout.split(
      id: "split", axis: .horizontal, ratio: 0.5,
      first: .pane(id: "session-pane", tabs: [sessionTab], active: sessionTab.id),
      second: .pane(id: "command-pane", tabs: [commandTab], active: commandTab.id))
    try model.panes.restore(layout)
    model.savePaneLayout()
    try await eventually { model.pendingSaves["/repo/a"] == nil }
    let save = try Releash_Client_V1_SaveWorkspaceStateRequest(
      serializedBytes: XCTUnwrap(http.requests("SaveWorkspaceState").last?.message))
    XCTAssertEqual(try PaneLayout(save.state.paneLayout), layout)

    // When / Then: reselecting and restarting both restore the actual pane view.
    model.selectWorktree("/repo/b")
    try await eventually { model.layoutLoaded }
    model.selectWorktree("/repo/a")
    try await eventually { model.layoutLoaded && model.layout == layout }
    for restarting in [false, true] {
      let restored: AppModel
      if restarting {
        await model.stop()
        restored = try await testModel(http)
      } else {
        restored = model
      }
      restored.selectWorktree("/repo/a")
      try await eventually { restored.layoutLoaded && restored.layout == layout }
      XCTAssertEqual(
        try restored.panes.layout().tabs.map(\.nodeID), [nil, "command-node"])
      http.state.withLock { $0.subscriptions.removeAll { $0.target == "node-detail" } }
      let host = await ViewHarness(WorkspacePaneView(model: restored))
      try await eventually {
        http.state.withLock { state in
          state.subscriptions.contains { $0.target == "session-tab" && $0.args == ["agent"] }
            && state.subscriptions.contains {
              $0.target == "node-detail" && $0.args == ["/repo/a", "command-node"]
            }
        }
      }
      try await eventuallyView(host) {
        host.texts().contains("Sessionは停止しています")
          && host.value(host.element("command-stdout")) == "saved command output"
      }
      XCTAssertTrue(
        restored.panes.controller.allTabIds.contains {
          restored.panes.controller.tab($0)?.title == "Restored Session"
        })
      host.close()
      if restarting { await restored.stop() }
    }
  }

  @MainActor func testCommandSubscriptionShowsEnvironmentFailureActionsAndPastAttempt() async throws
  {
    let http = TestHTTPClient()
    var detail = Releash_Client_V1_WorkspaceNodeDetailDto()
    detail.id = "root"
    detail.title = "Command"
    detail.executionID = "execution"
    detail.canAbort = true
    detail.capabilities.canRetry = true
    detail.errorReason = "current failure"
    detail.content.command.displayCommand = "printf current"
    detail.content.command.cwd = "/actual/worktree"
    detail.content.command.shell = "/bin/sh"
    detail.content.command.result.stdout = "current stdout"
    detail.content.command.result.stderr = "current stderr"
    detail.content.command.result.exitCode = 2
    detail.content.command.result.duration = 123
    var old = detail
    old.id = "old"
    old.title = "Old command"
    old.errorReason = "old failure"
    old.content.command.result.stdout = "old stdout"
    old.content.command.result.stderr = "old stderr"
    old.content.command.result.exitCode = 7
    var root = Releash_Client_V1_WorkspaceTreeItem()
    root.node.id = "root"
    root.node.contentKind.value = .command
    var attempt = Releash_Client_V1_WorkspacePastAttempt()
    attempt.node.id = "old"
    attempt.node.updatedAt = 12345
    attempt.node.errorReason = "old failure"
    root.node.pastAttempts.items = [attempt]
    old.updatedAt = 12345
    var summary = Releash_Client_V1_WorkspaceNodeAttemptDto()
    summary.id = old.id
    summary.title = old.title
    summary.updatedAt = old.updatedAt
    summary.errorReason = old.errorReason
    detail.pastAttempts = [summary]
    var tree = Releash_Client_V1_WorkspaceWorktreeList()
    tree.path = "/repo"
    tree.snapshot.nodes.items = [root]
    var repo = Releash_Client_V1_WorkspaceRepositoryList()
    repo.path = "/repo"
    repo.worktrees.items = [tree]
    http.state.withLock {
      $0.repositories = [repo]
      $0.nodeDetails = ["root": detail, "old": old]
    }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    let host = await ViewHarness(
      NodePane(model: model, path: "/repo", nodeID: "root", focused: true))
    try await eventuallyView(host) {
      host.value(host.element("command-stdout")) == "current stdout"
        && host.value(host.element("command-stderr")) == "current stderr"
    }
    for text in [
      "printf current", "/actual/worktree", "/bin/sh", "current failure", "Exit code: 2 · 123 ms",
    ] { XCTAssertTrue(host.texts().contains(text), text) }
    XCTAssertEqual(host.value(host.element("command-stdout")), "current stdout")
    XCTAssertEqual(host.value(host.element("command-stderr")), "current stderr")
    try await host.press("Retry")
    try await host.press("Abort")
    try await eventually {
      !http.requests("RetryWorkspaceNode").isEmpty && !http.requests("AbortWorkflow").isEmpty
    }
    let retry = try Releash_Client_V1_RetryWorkspaceNodeRequest(
      serializedBytes: XCTUnwrap(http.requests("RetryWorkspaceNode").last?.message))
    let abort = try Releash_Client_V1_AbortWorkflowRequest(
      serializedBytes: XCTUnwrap(http.requests("AbortWorkflow").last?.message))
    XCTAssertEqual(retry.nodeID, "root")
    XCTAssertEqual(abort.executionID, "execution")
    let label = "\(Date(timeIntervalSince1970: 12345).formatted()) · old failure"
    try await host.press(label)
    try await eventually { (try? model.panes.layout().tabs.last?.nodeID) == "old" }
    let oldHost = await ViewHarness(
      NodePane(model: model, path: "/repo", nodeID: "old", focused: true))
    try await eventuallyView(oldHost) {
      oldHost.value(oldHost.element("command-stdout")) == "old stdout"
        && oldHost.value(oldHost.element("command-stderr")) == "old stderr"
    }
    XCTAssertEqual(oldHost.value(oldHost.element("command-stdout")), "old stdout")
    XCTAssertEqual(oldHost.value(oldHost.element("command-stderr")), "old stderr")
    XCTAssertTrue(oldHost.texts().contains("Exit code: 7 · 123 ms"))
    oldHost.close()
    detail.canAbort = false
    detail.capabilities.canRetry = false
    var payload = Releash_Client_V1_StatePayload()
    payload.nodeDetail.value = detail
    try http.publish(payload, target: "node-detail")
    try await eventuallyView(host) { host.element("Retry") == nil && host.element("Abort") == nil }
    host.close()
    await model.stop()
  }
}

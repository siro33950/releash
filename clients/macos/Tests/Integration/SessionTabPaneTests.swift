import AppKit
import Bonsplit
import Connect
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class SessionTabPaneTests: XCTestCase {
  @MainActor func test_会話タブ購読_利用可否未設定で端末と購読を解放する() async throws {
    // Given
    let http = TestHTTPClient()
    var session = Releash_Client_V1_AgentSessionItemDto()
    session.id = "agent"
    session.availability = .open
    var node = Releash_Client_V1_WorkspaceNodeDetailDto()
    node.id = "node"
    node.content.session.sessionID = "agent"
    http.state.withLock {
      var snapshot = Releash_Client_V1_SessionTabSnapshot()
      snapshot.session = session
      snapshot.session.workspaceWorktreePath = "/repo"
      snapshot.node = node
      $0.sessionTabs["agent"] = snapshot
    }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    // When
    model.openNode(.session, id: "agent", title: "Session")
    let tab = try XCTUnwrap(model.panes.controller.allTabIds.first)
    let host = await ViewHarness(
      AnyView(SessionTabPane(model: model, tabID: tab, sessionID: "agent", focused: true)))
    // Then
    try await eventuallyView(host) { host.element("terminal:agent") != nil }
    session.clearAvailability()
    var payload = Releash_Client_V1_StatePayload()
    payload.sessionTab.session = session
    payload.sessionTab.session.workspaceWorktreePath = "/repo"
    payload.sessionTab.node = node
    // When
    try http.publish(payload, target: "session-tab")
    // Then
    try await eventuallyView(host) { host.element("terminal:agent") == nil }
    try await eventually {
      model.terminalAttachment(path: "/repo", id: "agent", agent: true) == nil
    }
    let subscription = try XCTUnwrap(
      http.state.withLock {
        $0.subscriptions.first { $0.target == "session-tab" && $0.args == ["agent"] }
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

  @MainActor func test_会話タブ購読_状態別の表示と再開失敗からの復旧を反映する() async throws {
    for (availability, canResume, message) in [
      (Releash_Client_V1_AgentSessionAvailability.paused, false, "Sessionは停止しています"),
      (.paused, true, "Sessionは停止しています"), (.indeterminate, true, "Sessionを開けるか判定できません"),
      (.archived, true, "SessionはArchiveされています"),
    ] {
      // Given
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
        var snapshot = Releash_Client_V1_SessionTabSnapshot()
        snapshot.session = session
        snapshot.session.workspaceWorktreePath = "/repo"
        snapshot.node = node
        $0.sessionTabs["agent"] = snapshot
        $0.terminalReplay = "resumed-output"
      }
      let model = try await testModel(http)
      model.selectWorktree("/repo")
      try await eventually { model.layoutLoaded }
      // When
      model.openNode(.session, id: "agent", title: "Session")
      let tab = try XCTUnwrap(model.panes.controller.allTabIds.first)
      let host = await ViewHarness(
        SessionTabPane(model: model, tabID: tab, sessionID: "agent", focused: true))
      // Then
      try await eventuallyView(host) { host.texts().contains(message) }
      XCTAssertNil(host.element("terminal:agent"))
      XCTAssertEqual(host.element("Resume") != nil, availability == .paused && canResume)
      try await eventually { !http.requests("OpenAgentSession").isEmpty }
      if availability == .paused && canResume {
        http.state.withLock { $0.failedMethod = "ResumeWorkspaceSessionNode" }
        // When
        try await host.press("Resume")
        // Then
        try await eventually { model.failure == "ResumeWorkspaceSessionNode failed" }
        XCTAssertNil(host.element("terminal:agent"))
        XCTAssertTrue(host.texts().contains(message))
        http.state.withLock { $0.failedMethod = nil }
        // When
        try await host.press("Resume")
        try await eventually { http.requests("ResumeWorkspaceSessionNode").count == 2 }
        let request = try Releash_Client_V1_ResumeWorkspaceSessionNodeRequest(
          serializedBytes: XCTUnwrap(http.requests("ResumeWorkspaceSessionNode").last?.message))
        XCTAssertEqual(request.nodeID, "node")
        XCTAssertEqual(request.worktreePath, "/repo")
        XCTAssertNil(host.element("terminal:agent"))
        var payload = Releash_Client_V1_StatePayload()
        session.availability = .open
        payload.sessionTab.session = session
        payload.sessionTab.session.workspaceWorktreePath = "/repo"
        payload.sessionTab.node = node
        // When
        try http.publish(payload, target: "session-tab")
        // Then
        try await eventuallyView(host) { host.element("terminal:agent") != nil }
        try await eventually {
          http.state.withLock {
            $0.subscriptions.contains { $0.target == "terminal" && $0.args == ["/repo", "agent"] }
          }
        }
        // Then
        try await eventuallyView(host) { host.texts().contains { $0.contains("resumed-output") } }
        XCTAssertEqual(http.requests("OpenAgentSession").count, 1)
        payload.sessionTab = .init()
        // When
        try http.publish(payload, target: "session-tab")
        // Then
        try await eventuallyView(host) { host.element("terminal:agent") == nil }
        try await eventually {
          model.terminalAttachment(path: "/repo", id: "agent", agent: true) == nil
        }
      }
      host.close()
      await model.stop()
    }
  }

  @MainActor func test_会話タブ購読_idだけで名前と利用可否を更新し端末を解放する() async throws {
    // Given
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
    // When
    model.openNode(.session, id: "agent", title: "Session")
    let tab = try XCTUnwrap(model.panes.controller.allTabIds.first)
    let host = await ViewHarness(
      AnyView(SessionTabPane(model: model, tabID: tab, sessionID: "agent", focused: true)))
    defer { host.close() }
    // Then
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
      // Given
      snapshot.session.availability = availability
      snapshot.node.title = "After"
      snapshot.node.classification.value = .idle
      var payload = Releash_Client_V1_StatePayload()
      payload.sessionTab = snapshot
      // When
      try http.publish(payload, target: "session-tab")
      // Then
      try await eventuallyView(host) { host.texts().contains(message) }
      XCTAssertNil(host.element("terminal:agent"))
      XCTAssertEqual(model.panes.controller.tab(tab)?.title, "After")
      try await eventually {
        model.terminalAttachment(path: "/repo", id: "agent", agent: true) == nil
      }
    }
    await model.stop()
  }
  @MainActor func test_会話タブメニュー_改名と整理確認と取消と操作制限を反映する()
    async throws
  {
    // Given
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
    // When
    model.openNode(.session, id: "agent", title: "Session")
    let host = await ViewHarness(WorkspacePaneView(model: model))
    defer { host.close() }
    // Then
    try await eventuallyView(host) { host.element("Session title") != nil }
    // When
    try await host.menu("Session title", selecting: "Rename") {
      // Then
      XCTAssertTrue($0.contains("Rename"), "Menu: \($0)")
    }
    // Then
    try await eventuallyView(host) { host.element("保存") != nil }
    // When
    try await host.setTextField("Renamed")
    try await host.pressAlertButton("保存")
    // Then
    try await eventually { http.requests("RenameWorkspaceSessionNode").count == 1 }
    try await eventuallyView(host) { host.element("保存") == nil }
    // When
    try await host.menu("Session title", selecting: "Archive") {
      // Then
      XCTAssertTrue($0.contains("Archive"))
    }
    // Then
    try await eventuallyView(host) { host.element("Abort and Archive") != nil }
    // When
    try await host.pressAlertButton("キャンセル")
    // Then
    XCTAssertTrue(http.requests("ArchiveWorkspaceWorkflowExecution").isEmpty)
    try await eventuallyView(host) { host.element("Abort and Archive") == nil }
    // When
    try await host.menu("Session title", selecting: "Archive") {
      // Then
      XCTAssertTrue($0.contains("Archive"))
    }
    // Then
    try await eventuallyView(host) { host.element("Abort and Archive") != nil }
    // When
    try await host.pressAlertButton("Abort and Archive")
    // Then
    try await eventually { http.requests("ArchiveWorkspaceWorkflowExecution").count == 1 }
    try await eventuallyView(host) { host.element("Abort and Archive") == nil }
    // Given
    snapshot.node.canAbort = false
    var payload = Releash_Client_V1_StatePayload()
    payload.sessionTab = snapshot
    // When
    try http.publish(payload, target: "session-tab")
    // Then
    try await eventually { model.panes.sessionSnapshots.values.first?.node.canAbort == false }
    // When
    try await host.menu("Session title", selecting: "Archive") {
      // Then
      XCTAssertTrue($0.contains("Archive"))
    }
    // Then
    try await eventually { http.requests("ArchiveWorkspaceWorkflowExecution").count == 2 }
    // Given
    snapshot.session.operations.canArchive = false
    payload.sessionTab = snapshot
    // When
    try http.publish(payload, target: "session-tab")
    // Then
    try await eventually {
      model.panes.sessionSnapshots.values.first?.session.operations.canArchive == false
    }
    // When
    try await host.menu("Session title", selecting: "Rename") {
      // Then
      XCTAssertTrue($0.contains("Rename"), "Menu: \($0)")
      XCTAssertFalse($0.contains("Archive"))
    }
    // Then
    try await eventuallyView(host) { host.element("保存") != nil }
    // When
    try await host.pressAlertButton("キャンセル")
    // Then
    XCTAssertEqual(http.requests("RenameWorkspaceSessionNode").count, 1)
    await model.stop()
  }
  @MainActor func test_会話タブ_起動失敗は利用可能な購読値の受信で消える() async throws {
    // Given
    let http = TestHTTPClient()
    var snapshot = Releash_Client_V1_SessionTabSnapshot()
    snapshot.session.id = "agent"
    snapshot.session.workspaceWorktreePath = "/repo"
    snapshot.session.availability = .paused
    http.state.withLock {
      $0.sessionTabs["agent"] = snapshot
      $0.failedMethod = "OpenAgentSession"
    }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    // When
    model.openNode(.session, id: "agent", title: "Session")
    let tab = try XCTUnwrap(model.panes.controller.allTabIds.first)
    // When
    let host = await ViewHarness(
      AnyView(SessionTabPane(model: model, tabID: tab, sessionID: "agent", focused: true)))
    defer { host.close() }
    // Then
    try await eventuallyView(host) { host.texts().contains("OpenAgentSession failed") }
    // Given
    var payload = Releash_Client_V1_StatePayload()
    snapshot.node.title = "Updated"
    payload.sessionTab = snapshot
    // When
    try http.publish(payload, target: "session-tab")
    // Then
    try await eventually { model.panes.controller.tab(tab)?.title == "Updated" }
    XCTAssertTrue(host.texts().contains("OpenAgentSession failed"))
    // Given
    snapshot.session.availability = .open
    payload.sessionTab = snapshot
    // When
    try http.publish(payload, target: "session-tab")
    // Then
    try await eventuallyView(host) {
      host.element("terminal:agent") != nil && !host.texts().contains("OpenAgentSession failed")
    }
    await model.stop()
  }

  @MainActor func test_会話タブ_同じ状態の再表示で起動が成功すると以前の失敗を消す() async throws {
    let http = TestHTTPClient()
    var snapshot = Releash_Client_V1_SessionTabSnapshot()
    snapshot.session.id = "agent"
    snapshot.session.workspaceWorktreePath = "/repo"
    snapshot.session.availability = .paused
    http.state.withLock {
      $0.sessionTabs["agent"] = snapshot
      $0.failedMethod = "OpenAgentSession"
    }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    model.openNode(.session, id: "agent", title: "Session")
    let tab = try XCTUnwrap(model.panes.controller.allTabIds.first)
    let host = await ViewHarness(
      AnyView(SessionTabPane(model: model, tabID: tab, sessionID: "agent", focused: true)))
    defer { host.close() }
    try await eventuallyView(host) { host.texts().contains("OpenAgentSession failed") }
    let content = try XCTUnwrap(host.window.contentView as? NSHostingView<AnyView>)
    host.window.contentView = NSView()
    try await eventually { model.panes.sessionSnapshots[tab] == nil }
    http.state.withLock {
      $0.failedMethod = nil
      $0.deferOpenSession = true
    }
    host.window.contentView = content
    try await eventuallyView(host) {
      http.state.withLock { $0.pendingOpenSession != nil }
        && host.texts().contains("OpenAgentSession failed")
    }
    let respond = try XCTUnwrap(http.state.withLock { $0.pendingOpenSession })
    respond(
      HTTPResponse(
        code: .ok, headers: ["content-type": ["application/proto"]], message: Data(),
        trailers: [:], error: nil, tracingInfo: .init(httpStatus: 200)))
    try await eventuallyView(host) {
      host.texts().contains("Sessionは停止しています")
        && !host.texts().contains("OpenAgentSession failed")
    }
    XCTAssertEqual(http.requests("OpenAgentSession").count, 2)
    await model.stop()
  }

  @MainActor func test_準備中の会話タブ_失敗理由を表示し紐付け後も同じタブで会話を表示する() async throws {
    // Given
    let http = TestHTTPClient()
    var detail = Releash_Client_V1_WorkspaceNodeDetailDto()
    detail.id = "node"
    detail.content.session = Releash_Client_V1_WorkspaceSessionNodeContentDto()
    http.state.withLock { $0.nodeDetails["node"] = detail }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    var root = Releash_Client_V1_WorkspaceTreeItem()
    root.node.id = "node"
    root.node.title = "Preparing"
    root.node.contentKind.value = .session
    // When
    model.openRoot(root)
    let tab = try XCTUnwrap(model.panes.controller.allTabIds.first)
    let host = await ViewHarness(WorkspacePaneView(model: model))
    defer { host.close() }
    // Then
    try await eventuallyView(host) { host.texts().contains("Sessionを準備しています") }
    XCTAssertEqual(try model.panes.layout().tabs.first?.nodeID, "node")
    // Given
    detail.errorReason = "Provider could not start"
    var payload = Releash_Client_V1_StatePayload()
    payload.nodeDetail.value = detail
    // When
    try http.publish(payload, target: "node-detail")
    // Then
    try await eventuallyView(host) { host.texts().contains("Provider could not start") }
    // Given
    var snapshot = Releash_Client_V1_SessionTabSnapshot()
    snapshot.session.id = "agent"
    snapshot.session.workspaceWorktreePath = "/repo"
    snapshot.session.availability = .open
    snapshot.node.title = "Ready"
    snapshot.node.classification.value = .active
    http.state.withLock { $0.sessionTabs["agent"] = snapshot }
    // Given
    detail.content.session.sessionID = "agent"
    payload.nodeDetail.value = detail
    // When
    try http.publish(payload, target: "node-detail")
    // Then
    try await eventuallyView(host) { host.element("terminal:agent") != nil }
    XCTAssertEqual(http.requests("OpenAgentSession").count, 1)
    XCTAssertFalse(
      http.state.withLock { $0.subscriptions.contains { $0.target == "agent-session" } })
    XCTAssertEqual(model.panes.controller.allTabIds, [tab])
    XCTAssertEqual(model.panes.controller.tab(tab)?.title, "Ready")
    XCTAssertEqual(try model.panes.layout().tabs.first?.agentSessionID, "agent")
    XCTAssertNil(try model.panes.layout().tabs.first?.nodeID)
    try await eventually { http.requests("SaveWorkspaceState").count > 0 }
    let saved = try Releash_Client_V1_SaveWorkspaceStateRequest(
      serializedBytes: XCTUnwrap(http.requests("SaveWorkspaceState").last?.message))
    XCTAssertEqual(try PaneLayout(saved.state.paneLayout).tabs.first?.agentSessionID, "agent")
    let subscriptions = http.state.withLock { $0.subscriptions }
    let preparing = try XCTUnwrap(
      subscriptions.first { $0.target == "node-detail" && $0.args == ["/repo", "node"] })
    let session = try XCTUnwrap(
      subscriptions.first { $0.target == "session-tab" && $0.args == ["agent"] })
    let terminal = try XCTUnwrap(
      subscriptions.first { $0.target == "terminal" && $0.args == ["/repo", "agent"] })
    func stopped(_ id: String) -> Bool {
      http.requests("StopStateSubscription").contains {
        (try? Releash_Client_V1_StopStateSubscriptionRequest(serializedBytes: $0.message ?? Data())
          .subscriptionID) == id
      }
    }
    // Then
    try await eventually { stopped(preparing.subscriptionID) }
    XCTAssertNotNil(model.terminalAttachment(path: "/repo", id: "agent", agent: true))
    // When
    model.panes.controller.closeTab(tab)
    // Then
    try await eventually {
      stopped(session.subscriptionID) && stopped(terminal.subscriptionID)
        && model.terminalAttachment(path: "/repo", id: "agent", agent: true) == nil
    }
    await model.stop()
  }

  @MainActor func test_準備中の会話タブ_一覧の紐付けが先に届いても同じタブを保存し購読する() async throws {
    // Given
    let http = TestHTTPClient()
    var detail = Releash_Client_V1_WorkspaceNodeDetailDto()
    detail.id = "node"
    detail.content.session = Releash_Client_V1_WorkspaceSessionNodeContentDto()
    http.state.withLock { $0.nodeDetails["node"] = detail }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    var root = Releash_Client_V1_WorkspaceTreeItem()
    root.node.id = "node"
    root.node.contentKind.value = .session
    // When
    model.openRoot(root)
    let tab = try XCTUnwrap(model.panes.controller.allTabIds.first)
    let host = await ViewHarness(WorkspacePaneView(model: model))
    defer { host.close() }
    // Then
    try await eventuallyView(host) { host.texts().contains("Sessionを準備しています") }
    // Given
    var snapshot = Releash_Client_V1_SessionTabSnapshot()
    snapshot.session.id = "agent"
    snapshot.session.workspaceWorktreePath = "/repo"
    snapshot.session.availability = .open
    snapshot.node.title = "Ready"
    http.state.withLock { $0.sessionTabs["agent"] = snapshot }
    root.node.sessionID = "agent"
    // When
    model.openRoot(root)
    // Then
    try await eventuallyView(host) { host.element("terminal:agent") != nil }
    XCTAssertEqual(model.panes.controller.allTabIds, [tab])
    XCTAssertEqual(
      model.panes.controller.selectedTab(inPane: model.panes.controller.allPaneIds[0])?.id, tab)
    XCTAssertEqual(model.panes.controller.tab(tab)?.title, "Ready")
    XCTAssertEqual(try model.panes.layout().tabs.first?.agentSessionID, "agent")
    XCTAssertNil(try model.panes.layout().tabs.first?.nodeID)
    // Given
    detail.content.session.sessionID = "agent"
    var payload = Releash_Client_V1_StatePayload()
    payload.nodeDetail.value = detail
    // When
    try http.publish(payload, target: "node-detail")
    model.openRoot(root)
    // Then
    XCTAssertEqual(model.panes.controller.allTabIds, [tab])
    try await eventually { http.requests("SaveWorkspaceState").count > 0 }
    let saved = try Releash_Client_V1_SaveWorkspaceStateRequest(
      serializedBytes: XCTUnwrap(http.requests("SaveWorkspaceState").last?.message))
    XCTAssertEqual(try PaneLayout(saved.state.paneLayout).tabs.first?.agentSessionID, "agent")
    XCTAssertTrue(
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "session-tab" && $0.args == ["agent"] }
      })
    await model.stop()
  }

  @MainActor func test_準備中の会話タブ_操作応答と通知の順序によらず同じ配置を購読し保存する() async throws {
    for history in [false, true] {
      for notificationFirst in [false, true] {
        // Given
        let http = TestHTTPClient()
        var detail = Releash_Client_V1_WorkspaceNodeDetailDto()
        detail.id = "session-tree"
        detail.nodeExecutionID = "internal-session-node"
        detail.content.session = Releash_Client_V1_WorkspaceSessionNodeContentDto()
        var snapshot = Releash_Client_V1_SessionTabSnapshot()
        snapshot.session.id = "agent-session"
        snapshot.session.availability = .open
        snapshot.session.workspaceWorktreePath = "/repo"
        snapshot.node.title = "Ready"
        http.state.withLock {
          $0.nodeDetails["session-tree"] = detail
          $0.sessionTabs["agent-session"] = snapshot
        }
        var archived = Releash_Client_V1_AgentSessionItemDto()
        archived.id = "agent-session"
        archived.provider.value = .codex
        archived.providerSessionID = "history"
        var worktree = Releash_Client_V1_WorkspaceWorktreeList()
        worktree.path = "/repo"
        worktree.snapshot.archivedSessions.items = [archived]
        var repository = Releash_Client_V1_WorkspaceRepositoryList()
        repository.path = "/repo"
        repository.worktrees.items = [worktree]
        http.state.withLock { $0.repositories = [repository] }
        let model = try await testModel(http)
        model.selectWorktree("/repo")
        try await eventually { model.layoutLoaded }
        var root = Releash_Client_V1_WorkspaceTreeItem()
        root.node.id = "session-tree"
        root.node.contentKind.value = .session
        model.openRoot(root)
        let tab = try XCTUnwrap(model.panes.controller.allTabIds.first)
        let observer = TabCreationObserver()
        model.panes.controller.delegate = observer
        let host = await ViewHarness(WorkspacePaneView(model: model))
        try await eventuallyView(host) { host.texts().contains("Sessionを準備しています") }
        detail.content.session.sessionID = "agent-session"
        var payload = Releash_Client_V1_StatePayload()
        payload.nodeDetail.value = detail
        // When
        if notificationFirst {
          try http.publish(payload, target: "node-detail")
          try await eventuallyView(host) { host.element("terminal:agent-session") != nil }
        }
        if history {
          let historyHost = await ViewHarness(SessionHistoryView(model: model, path: "/repo"))
          try await eventuallyView(historyHost) { historyHost.element("Codex · history") != nil }
          try await historyHost.press("Codex · history")
          try await eventuallyView(host) { host.element("terminal:agent-session") != nil }
          historyHost.close()
        } else {
          try await model.startSession(provider: "codex", path: "/repo")
        }
        try await eventuallyView(host) { host.element("terminal:agent-session") != nil }
        if !notificationFirst { try http.publish(payload, target: "node-detail") }
        // Then
        XCTAssertEqual(observer.created, 0)
        XCTAssertEqual(model.panes.controller.allTabIds, [tab])
        XCTAssertEqual(try model.panes.layout().tabs.first?.agentSessionID, "agent-session")
        XCTAssertNil(try model.panes.layout().tabs.first?.nodeID)
        try await eventually {
          guard let data = http.requests("SaveWorkspaceState").last?.message,
            let saved = try? Releash_Client_V1_SaveWorkspaceStateRequest(serializedBytes: data)
          else { return false }
          return (try? PaneLayout(saved.state.paneLayout).tabs.first?.agentSessionID)
            == "agent-session"
        }
        host.close()
        await model.stop()
      }
    }
  }
}

@MainActor private final class TabCreationObserver: @preconcurrency BonsplitDelegate {
  var created = 0
  func splitTabBar(
    _ controller: BonsplitController, didCreateTab tab: Bonsplit.Tab, inPane pane: PaneID
  ) {
    created += 1
  }
}

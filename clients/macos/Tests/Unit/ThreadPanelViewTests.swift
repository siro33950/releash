import Connect
import SwiftProtobuf
import XCTest

@testable import Releash

final class ThreadPanelViewTests: XCTestCase {
  @MainActor func test_Commentリンク_許可したschemeだけ外部で開き起動結果を返す() throws {
    for (address, allowed) in [
      ("https://example.com", true), ("HTTP://example.com", true),
      ("mailto:user@example.com", true), ("file:///tmp/test", false),
      ("javascript:alert(1)", false), ("tel:123", false), ("relative", false),
    ] {
      // Given
      let url = try XCTUnwrap(URL(string: address))
      var opened: [URL] = []
      // When
      let handled = openCommentLink(url) {
        opened.append($0)
        return true
      }
      // Then
      XCTAssertEqual(handled, allowed)
      XCTAssertEqual(opened, allowed ? [url] : [])
    }
    // Given
    let url = try XCTUnwrap(URL(string: "https://example.com"))
    // When
    let handled = openCommentLink(url) { _ in false }
    // Then
    XCTAssertFalse(handled)
  }

  func test_Thread表示_場所と行と解決済みを表す() {
    // Given
    var thread = testReviewThread("thread")
    // Then
    XCTAssertEqual(thread.pathLabel, "General")
    XCTAssertNil(thread.lineLabel)
    XCTAssertEqual(thread.compactLocationLabel, "General")
    XCTAssertFalse(thread.isResolved)
    // When
    thread.target.filePath = "src/main.rs"
    // Then
    XCTAssertEqual(thread.pathLabel, "src/main.rs")
    XCTAssertNil(thread.lineLabel)
    XCTAssertEqual(thread.compactLocationLabel, "main.rs")
    // When
    thread.target.lineNumber = 12
    thread.target.endLine = 12
    // Then
    XCTAssertEqual(thread.lineLabel, "L12")
    // When
    thread.target.endLine = 18
    thread.state.value = .resolved
    // Then
    XCTAssertEqual(thread.lineLabel, "L12–18")
    XCTAssertEqual(thread.compactLocationLabel, "main.rs L12–18")
    XCTAssertTrue(thread.isResolved)
  }

  func test_相対時刻_年と日と時間と分の区切りで表す() {
    // Given
    let now = Date(timeIntervalSince1970: 2_000_000_000)
    for (elapsed, expected) in [
      (-10.0, "たった今"), (0, "たった今"), (59, "たった今"), (60, "1 分前"), (3_599, "59 分前"),
      (3_600, "1 時間前"), (86_399, "23 時間前"), (86_400, "1 日前"), (31_535_999, "364 日前"),
      (31_536_000, "1 年前"),
    ] {
      // When
      let label = relativeTimeLabel(now.timeIntervalSince1970 - elapsed, now: now)
      // Then
      XCTAssertEqual(label, expected, "\(elapsed)")
    }
    XCTAssertEqual(relativeTimeLabel(.nan, now: now), "")
  }

  func test_差分の行_追加と削除と変更なしを記号と読み上げで表す() {
    // Given
    let added = Releash_Client_V1_ThreadDiffLine.with { $0.newLine = 3 }
    let removed = Releash_Client_V1_ThreadDiffLine.with { $0.oldLine = 3 }
    let context = Releash_Client_V1_ThreadDiffLine.with {
      $0.oldLine = 3
      $0.newLine = 4
    }
    // Then
    XCTAssertEqual([added.sign, removed.sign, context.sign], ["+", "-", " "])
    XCTAssertEqual(added.accessibilityDescription, "追加")
    XCTAssertEqual(removed.accessibilityDescription, "削除")
    XCTAssertEqual(context.accessibilityDescription, "")
    XCTAssertEqual(
      Releash_Client_V1_ThreadDiffLine.with {
        $0.newLine = 3
        $0.targeted = true
      }.accessibilityDescription, "追加、指摘された行")
  }

  @MainActor func test_Thread一覧_未解決と解決済みに分け無くなったThreadの選択と下書きを外す() {
    // Given
    let state = ThreadPanelState()
    let open = testReviewThread("open")
    let resolved = testReviewThread("resolved", resolved: true)
    state.receive(testThreadExcerpts([open, resolved]).items)
    XCTAssertTrue(state.selectedIDs.isEmpty)
    XCTAssertEqual(state.openItems.map(\.thread.id), ["open"])
    XCTAssertEqual(state.resolvedItems.map(\.thread.id), ["resolved"])
    // When
    state.checked = ["resolved", "open"]
    state.expanded = ["resolved"]
    state.drafts = ["open": "返信の下書き"]
    // Then
    XCTAssertEqual(state.selectedIDs, ["open", "resolved"])
    // When
    state.receive(testThreadExcerpts([resolved]).items)
    // Then
    XCTAssertEqual(state.selectedIDs, ["resolved"])
    XCTAssertEqual(state.checked, ["resolved"])
    XCTAssertEqual(state.expanded, ["resolved"])
    XCTAssertTrue(state.drafts.isEmpty)
  }

  @MainActor func test_送り先_選択worktreeの子孫を含むliveのSessionだけを表示する() {
    // Given
    let model = AppModel()
    var worktree = Releash_Client_V1_WorkspaceWorktreeList()
    worktree.path = "/repo"
    let view = ThreadPanelView(model: model, worktree: "/repo")
    var nodes: [Releash_Client_V1_WorkspaceTreeItem] = []
    for (index, presence) in [
      Releash_Client_V1_NodeProcessPresence.Value.live, .confirmedAbsent, .unknown,
    ].enumerated() {
      let id = "session-\(index)"
      var node = Releash_Client_V1_WorkspaceTreeItem()
      node.node.sessionID = id
      node.node.title = id
      node.node.processPresence.value = presence
      nodes.append(node)
    }
    var group = Releash_Client_V1_WorkspaceTreeItem()
    group.sequence.children.items = nodes
    var unknown = Releash_Client_V1_WorkspaceTreeItem()
    unknown.node.sessionID = "not-received"
    var command = Releash_Client_V1_WorkspaceTreeItem()
    command.node.processPresence.value = .live
    worktree.snapshot.nodes.items = [group, unknown, command]
    var other = worktree
    other.path = "/other"
    other.snapshot.nodes.items = [
      .with {
        $0.node.sessionID = "foreign"
        $0.node.processPresence.value = .live
      }
    ]
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.worktrees.items = [other, worktree]
    model.repositories = [repository]
    // Then
    XCTAssertEqual(view.openSessions.map(\.sessionID), ["session-0"])
    // When
    model.repositories = []
    // Then
    XCTAssertTrue(view.openSessions.isEmpty)
  }

  @MainActor func test_送り先_過去の試行にだけあるliveのSessionを含める() {
    // Given
    var current = Releash_Client_V1_WorkspaceTreeItem()
    current.node.sessionID = "current"
    current.node.processPresence.value = .confirmedAbsent
    var past = Releash_Client_V1_WorkspacePastAttempt()
    past.node.sessionID = "past"
    past.node.processPresence.value = .live
    past.node.children.items = [
      .with {
        $0.node.sessionID = "past-child"
        $0.node.processPresence.value = .live
      }
    ]
    var stopped = Releash_Client_V1_WorkspacePastAttempt()
    stopped.node.sessionID = "stopped"
    stopped.node.processPresence.value = .confirmedAbsent
    current.node.pastAttempts.items = [past, stopped]
    let model = AppModel()
    model.repositories = [
      .with {
        $0.worktrees.items = [
          .with {
            $0.path = "/repo"
            $0.snapshot.nodes.items = [current]
          }
        ]
      }
    ]
    let view = ThreadPanelView(model: model, worktree: "/repo")
    // Then
    XCTAssertEqual(view.openSessions.map(\.sessionID), ["past", "past-child"])
  }

  @MainActor func test_Thread操作_RPCの宛先と本文と送信IDを渡す() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let view = ThreadPanelView(model: model, worktree: "/repo")
    view.state.receive(
      testThreadExcerpts([testReviewThread("a"), testReviewThread("b", resolved: true)]).items)
    view.state.checked = ["a", "b"]
    var replies = 0
    var creations = 0
    // When
    view.reply(to: "a", content: "返信") { if $0 { replies += 1 } }
    view.resolve("a")
    view.createGeneral(content: "General本文") { if $0 { creations += 1 } }
    view.send(to: "session")
    // Then
    try await eventually {
      replies == 1 && creations == 1 && !http.requests("SendThreadsToSession").isEmpty
        && !http.requests("ResolveReviewThread").isEmpty
    }
    let reply = try Releash_Client_V1_AppendReviewCommentRequest(
      serializedBytes: XCTUnwrap(http.requests("AppendReviewComment").first?.message))
    XCTAssertEqual(reply.worktreeName, "/repo")
    XCTAssertEqual(reply.threadID, "a")
    XCTAssertEqual(reply.content, "返信")
    let resolve = try Releash_Client_V1_ResolveReviewThreadRequest(
      serializedBytes: XCTUnwrap(http.requests("ResolveReviewThread").first?.message))
    XCTAssertEqual(resolve.worktreeName, "/repo")
    XCTAssertEqual(resolve.threadID, "a")
    XCTAssertEqual(resolve.outcome, "resolved")
    XCTAssertEqual(resolve.summary, "Threadの面で解決")
    let create = try Releash_Client_V1_CreateReviewThreadRequest(
      serializedBytes: XCTUnwrap(http.requests("CreateReviewThread").first?.message))
    XCTAssertEqual(create.worktreeName, "/repo")
    XCTAssertEqual(create.content, "General本文")
    XCTAssertFalse(create.hasFilePath)
    XCTAssertFalse(create.hasLineNumber)
    let send = try Releash_Client_V1_SendThreadsToSessionRequest(
      serializedBytes: XCTUnwrap(http.requests("SendThreadsToSession").first?.message))
    XCTAssertEqual(send.agentSessionID, "session")
    XCTAssertEqual(send.threadIds.items, ["a", "b"])
    await model.stop()
  }

  @MainActor func test_Thread操作_失敗を表示し返信と作成の下書きを保持する() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let view = ThreadPanelView(model: model, worktree: "/repo")
    var accepted = false
    for method in ["AppendReviewComment", "CreateReviewThread", "ResolveReviewThread"] {
      http.state.withLock { $0.failedMethod = method }
      model.failure = nil
      // When
      switch method {
      case "AppendReviewComment": view.reply(to: "a", content: "本文") { accepted = $0 }
      case "CreateReviewThread": view.createGeneral(content: "本文") { accepted = $0 }
      default: view.resolve("a")
      }
      // Then
      try await eventually { model.failure == "\(method) failed" }
      XCTAssertFalse(accepted)
    }
    for code in [Code.failedPrecondition, .notFound] {
      http.state.withLock {
        $0.failedMethod = "SendThreadsToSession"
        $0.failedMethodCode = code
      }
      model.failure = nil
      // When
      view.send(to: "session")
      // Then
      try await eventually { model.failure == "SendThreadsToSession failed" }
    }
    await model.stop()
  }

  @MainActor func test_その場の入力_送れたら入力欄を空にし失敗と空白では残す() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let view = ThreadPanelView(model: model, worktree: "/repo")
    view.state.receive(testThreadExcerpts([testReviewThread("a")]).items)
    view.state.drafts["a"] = "返信"
    view.state.generalDraft = "General本文"
    // When
    view.sendReply("a")
    view.sendReply("a")
    view.addGeneral()
    // Then
    try await eventually {
      view.state.drafts["a"] == nil && view.state.generalDraft.isEmpty
        && view.state.sending.isEmpty && !view.state.creatingGeneral
    }
    XCTAssertEqual(http.requests("AppendReviewComment").count, 1)
    XCTAssertEqual(http.requests("CreateReviewThread").count, 1)
    // When
    view.state.drafts["a"] = "  "
    view.state.generalDraft = "\n"
    view.sendReply("a")
    view.addGeneral()
    // Then
    XCTAssertTrue(view.state.sending.isEmpty)
    XCTAssertFalse(view.state.creatingGeneral)
    // When
    http.state.withLock { $0.failedMethod = "AppendReviewComment" }
    view.state.drafts["a"] = "再送する返信"
    view.sendReply("a")
    // Then
    try await eventually { model.failure == "AppendReviewComment failed" }
    XCTAssertEqual(view.state.drafts["a"], "再送する返信")
    XCTAssertTrue(view.state.sending.isEmpty)
    await model.stop()
  }
}

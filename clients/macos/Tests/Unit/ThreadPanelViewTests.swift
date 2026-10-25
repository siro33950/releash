import Connect
import SwiftProtobuf
import XCTest

@testable import Releash

final class ThreadPanelViewTests: XCTestCase {
  func test_Thread行_位置と件数と最終発言と解決済みを表示する() {
    // Given
    var thread = testReviewThread("thread")
    // Then
    XCTAssertEqual(thread.locationLabel, "General")
    XCTAssertEqual(thread.commentCountLabel, "Comment 1件")
    XCTAssertFalse(thread.isResolved)
    // When
    thread.target.filePath = "src/main.rs"
    // Then
    XCTAssertEqual(thread.locationLabel, "src/main.rs")
    // When
    thread.target.lineNumber = 12
    thread.target.endLine = 12
    // Then
    XCTAssertEqual(thread.locationLabel, "src/main.rs:12")
    // When
    thread.target.endLine = 18
    thread.state.value = .resolved
    var reply = Releash_Client_V1_ReviewCommentDto()
    reply.author.displayName = "Agent"
    reply.createdAt = 1_700_000_100
    thread.comments.items.append(reply)
    // Then
    XCTAssertEqual(thread.locationLabel, "src/main.rs:12–18")
    XCTAssertEqual(thread.commentCountLabel, "Comment 2件")
    XCTAssertTrue(thread.isResolved)
    XCTAssertEqual(
      thread.lastCommentLabel,
      "Agent · \(Date(timeIntervalSince1970: reply.createdAt).formatted(date: .abbreviated, time: .shortened))"
    )
    // When
    thread.comments.items = []
    // Then
    XCTAssertEqual(thread.commentCountLabel, "Comment 0件")
    XCTAssertEqual(thread.lastCommentLabel, "")
  }

  @MainActor func test_Thread選択_状態を切り替えても保持し削除されたIDだけ外す() {
    // Given
    let state = ThreadPanelState()
    let open = testReviewThread("open")
    let resolved = testReviewThread("resolved", resolved: true)
    state.receive([open, resolved])
    XCTAssertTrue(state.selectedIDs.isEmpty)
    XCTAssertEqual(state.visibleThreads.map(\.id), ["open"])
    // When
    state.checked.insert("open")
    state.showResolved = true
    state.checked.insert("resolved")
    // Then
    XCTAssertEqual(state.visibleThreads.map(\.id), ["resolved"])
    XCTAssertEqual(state.selectedIDs, ["open", "resolved"])
    XCTAssertEqual(state.selectedIDs.count, 2)
    // When
    state.receive([resolved])
    // Then
    XCTAssertEqual(state.selectedIDs, ["resolved"])
    XCTAssertEqual(state.checked, ["resolved"])
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
    view.state.receive([testReviewThread("a"), testReviewThread("b", resolved: true)])
    view.state.checked = ["a", "b"]
    var replies = 0
    var creations = 0
    // When
    view.reply(to: "a", content: "返信") { replies += 1 }
    view.resolve("a")
    view.createGeneral(content: "General本文") { creations += 1 }
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
      case "AppendReviewComment": view.reply(to: "a", content: "本文") { accepted = true }
      case "CreateReviewThread": view.createGeneral(content: "本文") { accepted = true }
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
}

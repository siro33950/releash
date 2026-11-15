import AppKit
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class ThreadPanelViewTests: XCTestCase {
  private var savedDiffBase: Any?

  override func setUp() {
    savedDiffBase = UserDefaults.standard.object(forKey: "diffBase")
    UserDefaults.standard.set("head", forKey: "diffBase")
  }

  override func tearDown() {
    UserDefaults.standard.set(savedDiffBase, forKey: "diffBase")
  }

  @MainActor func test_Thread面_件数とカードとたたんだ解決済みを表示し表示で開く() async throws {
    // Given
    let http = TestHTTPClient()
    var open = testReviewThread("open")
    open.target.filePath = "server/src/main.rs"
    open.target.lineNumber = 2
    var agent = Releash_Client_V1_ReviewCommentDto()
    agent.id = "agent-reply"
    agent.content = "agentの返信"
    agent.author.kind.value = .agent
    agent.author.displayName = "claude"
    agent.author.model = "acceptance-model"
    agent.createdAt = 1_700_000_100
    open.comments.items.append(agent)
    var resolved = testReviewThread("resolved", resolved: true)
    resolved.target.filePath = "server/src/failure.rs"
    resolved.target.lineNumber = 40
    resolved.resolve.actor.displayName = "Human"
    resolved.resolve.resolvedAt = 1_700_000_200
    http.state.withLock {
      $0.reviewThreads["/repo"] = .with { $0.items = [open, resolved] }
      $0.threadLines["open"] = [
        .with {
          $0.oldLine = 1
          $0.newLine = 1
          $0.text = "    context"
        },
        .with {
          $0.newLine = 2
          $0.text = "added"
          $0.targeted = true
        },
      ]
    }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    let host = await ViewHarness(
      RightPanelView(expanded: .constant(true), model: model),
      size: NSSize(width: 480, height: 900))
    // When
    try await host.press("right-panel:threads")
    // Then
    try await eventuallyView(host) { host.element("thread:open") != nil }
    for id in [
      "thread-counts", "thread-selected-count", "thread-send", "thread-check:open",
      "thread-path:open", "thread-line:open", "thread-diff:open", "thread-body:open",
      "thread-comment:agent-reply", "thread-comment-model:agent-reply",
      "thread-reply-content:open", "thread-resolve:open", "thread:resolved",
      "thread-resolved:resolved", "thread-toggle:resolved", "thread-general-content",
      "thread-general-create",
    ] {
      XCTAssertNotNil(host.element(id), id)
    }
    let texts = host.texts()
    for text in [
      "未解決 1 · 解決済み 1", "0 件を選択", "server/src/main.rs", "L2", "agentの返信", "acceptance-model",
      "failure.rs L40",
    ] {
      XCTAssertTrue(texts.contains(text), text)
    }
    XCTAssertTrue(texts.contains { $0.contains("確認") })
    XCTAssertTrue(texts.contains { $0.contains("let value = 1") })
    XCTAssertTrue(texts.contains { $0.hasPrefix("Human が解決にしました · ") })
    XCTAssertEqual(host.element("thread-diff-line:open:0")?.label, "    context")
    XCTAssertEqual(host.element("thread-diff-line:open:0")?.value, "")
    XCTAssertEqual(host.element("thread-diff-line:open:1")?.label, "added")
    XCTAssertEqual(host.element("thread-diff-line:open:1")?.value, "追加、指摘された行")
    for id in [
      "thread-check:resolved", "thread-body:resolved", "thread-reply-send:open",
      "thread-diff:resolved",
    ] {
      XCTAssertNil(host.element(id), id)
    }
    XCTAssertFalse(try XCTUnwrap(host.element("thread-send")).enabled)
    XCTAssertFalse(try XCTUnwrap(host.element("thread-general-create")).enabled)
    // When
    try await host.press("thread-check:open")
    // Then
    try await eventuallyView(host) { host.texts().contains("1 件を選択") }
    // When
    try await host.press("thread-toggle:resolved")
    // Then
    try await eventuallyView(host) { host.element("thread-body:resolved") != nil }
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains {
          $0.target == "review-threads" && $0.args == ["/repo", "head", "thread=resolved"]
        }
      }
    }
    XCTAssertNotNil(host.element("thread-check:resolved"))
    XCTAssertNotNil(host.element("thread-resolution:resolved"))
    XCTAssertNil(host.element("thread-reply-content:resolved"))
    XCTAssertNil(host.element("thread-resolve:resolved"))
    // When
    try await host.press("thread-check:resolved")
    // Then
    try await eventuallyView(host) { host.texts().contains("2 件を選択") }
    XCTAssertFalse(try XCTUnwrap(host.element("thread-send")).enabled)
    // When
    try await host.press("thread-toggle:resolved")
    // Then
    try await eventuallyView(host) { host.element("thread-body:resolved") == nil }
    XCTAssertTrue(host.texts().contains("2 件を選択"))
    host.close()
    await model.stop()
  }

  @MainActor func test_その場の入力_返信とGeneralの追加を入力欄から送り送れたら空にする() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.reviewThreads["/repo"] = .with { $0.items = [testReviewThread("open")] }
    }
    let model = try await testModel(http)
    let host = await ViewHarness(AnyView(ThreadPanelView(model: model, worktree: "/repo")))
    try await eventuallyView(host) { host.element("thread-reply-content:open") != nil }
    XCTAssertEqual(host.element("thread-resolve:open")?.enabled, true)
    // When
    try await host.focusTextField(index: 0)
    try await host.setTextField("その場の返信\n2 行目")
    // Then
    try await eventuallyView(host) { host.element("thread-reply-send:open") != nil }
    XCTAssertEqual(host.element("thread-resolve:open")?.enabled, false)
    // When
    try await host.press("thread-reply-send:open")
    // Then
    try await eventually { !http.requests("AppendReviewComment").isEmpty }
    let reply = try Releash_Client_V1_AppendReviewCommentRequest(
      serializedBytes: XCTUnwrap(http.requests("AppendReviewComment").first?.message))
    XCTAssertEqual(reply.threadID, "open")
    XCTAssertEqual(reply.content, "その場の返信\n2 行目")
    try await eventuallyView(host) { host.element("thread-reply-send:open") == nil }
    XCTAssertEqual(host.element("thread-reply-content:open")?.value, "")
    XCTAssertEqual(host.element("thread-resolve:open")?.enabled, true)
    // When
    try await host.focusTextField(index: 1)
    try await host.setTextField("その場のGeneral")
    try await eventuallyView(host) { host.element("thread-general-create")?.enabled == true }
    try await host.confirm("thread-general-content")
    // Then
    try await eventually { !http.requests("CreateReviewThread").isEmpty }
    let create = try Releash_Client_V1_CreateReviewThreadRequest(
      serializedBytes: XCTUnwrap(http.requests("CreateReviewThread").first?.message))
    XCTAssertEqual(create.content, "その場のGeneral")
    XCTAssertFalse(create.hasFilePath)
    try await eventuallyView(host) { host.element("thread-general-create")?.enabled == false }
    host.close()
    await model.stop()
  }

  @MainActor func test_面を離れる_書きかけの返信とチェックを戻ったときに保つ() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.reviewThreads["/repo"] = .with { $0.items = [testReviewThread("open")] }
    }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    let host = await ViewHarness(
      RightPanelView(expanded: .constant(true), model: model),
      size: NSSize(width: 480, height: 900))
    try await host.press("right-panel:threads")
    try await eventuallyView(host) { host.element("thread:open") != nil }
    model.threadPanelState("/repo").drafts["open"] = "書きかけ"
    try await host.press("thread-check:open")
    try await eventuallyView(host) {
      host.texts().contains("1 件を選択") && host.element("thread-reply-send:open") != nil
    }
    // When
    try await host.press("right-panel:roots")
    try await eventuallyView(host) { host.element("thread:open") == nil }
    try await host.press("right-panel:threads")
    // Then
    try await eventuallyView(host) {
      host.element("thread-reply-content:open")?.value == "書きかけ"
    }
    XCTAssertTrue(host.texts().contains("1 件を選択"))
    XCTAssertNotNil(host.element("thread-reply-send:open"))
    host.close()
    await model.stop()
  }

  @MainActor func test_比較の基準_設定を変えると新しい基準で購読し直す() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let host = await ViewHarness(AnyView(ThreadPanelView(model: model, worktree: "/repo")))
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "review-threads" && $0.args == ["/repo", "head"] }
      }
    }
    // When
    UserDefaults.standard.set("branch-base", forKey: "diffBase")
    // Then
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains {
          $0.target == "review-threads" && $0.args == ["/repo", "branch-base"]
        }
      }
    }
    host.close()
    await model.stop()
  }

  @MainActor func test_差分の失敗_読めなかった理由をカードに出しCommentは出す() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let host = await ViewHarness(AnyView(ThreadPanelView(model: model, worktree: "/repo")))
    try await eventually {
      http.state.withLock { $0.subscriptions.contains { $0.target == "review-threads" } }
    }
    var payload = Releash_Client_V1_StatePayload()
    payload.threadExcerpts.items = [
      .with {
        $0.thread = testReviewThread("broken")
        $0.readError = "read failed"
      }
    ]
    // When
    try http.publish(payload, target: "review-threads", args: ["/repo", "head"])
    // Then
    try await eventuallyView(host) { host.element("thread-diff-error:broken") != nil }
    XCTAssertTrue(host.texts().contains("差分を読めません: read failed"))
    XCTAssertNotNil(host.element("thread-body:broken"))
    XCTAssertNil(host.element("thread-diff:broken"))
    host.close()
    await model.stop()
  }

  @MainActor func test_Thread購読_追加と返信と解決と削除を反映して終了時に購読を解放する() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let host = await ViewHarness(
      AnyView(ThreadPanelView(model: model, worktree: "/repo")))
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "review-threads" && $0.args == ["/repo", "head"] }
      }
    }
    var thread = testReviewThread("created")
    var payload = Releash_Client_V1_StatePayload()
    payload.threadExcerpts = testThreadExcerpts([thread])
    // When
    try http.publish(payload, target: "review-threads", args: ["/repo", "head"])
    // Then
    try await eventuallyView(host) { host.element("thread:created") != nil }
    XCTAssertTrue(host.texts().contains("未解決 1 · 解決済み 0"))
    // When
    var reply = thread.comments.items[0]
    reply.id = "reply"
    reply.content = "返信本文"
    thread.comments.items.append(reply)
    payload.threadExcerpts = testThreadExcerpts([thread])
    try http.publish(payload, target: "review-threads", args: ["/repo", "head"])
    // Then
    try await eventuallyView(host) { host.texts().contains("返信本文") }
    XCTAssertNotNil(host.element("thread-comment:reply"))
    // When
    thread.state.value = .resolved
    payload.threadExcerpts = testThreadExcerpts([thread])
    try http.publish(payload, target: "review-threads", args: ["/repo", "head"])
    // Then
    try await eventuallyView(host) { host.element("thread-resolved:created") != nil }
    XCTAssertTrue(host.texts().contains("未解決 0 · 解決済み 1"))
    XCTAssertNil(host.element("thread-reply-content:created"))
    XCTAssertNil(host.element("thread-body:created"))
    // When
    payload.threadExcerpts = testThreadExcerpts([])
    try http.publish(payload, target: "review-threads", args: ["/repo", "head"])
    // Then
    try await eventuallyView(host) { host.element("thread:created") == nil }
    let subscription = try XCTUnwrap(
      http.state.withLock { $0.subscriptions.first { $0.target == "review-threads" } })
    let hostingView = try XCTUnwrap(host.window.contentView as? NSHostingView<AnyView>)
    // When
    hostingView.rootView = AnyView(EmptyView())
    // Then
    try await eventually {
      http.requests("StopStateSubscription").contains {
        (try? Releash_Client_V1_StopStateSubscriptionRequest(serializedBytes: $0.message ?? Data())
          .subscriptionID) == subscription.subscriptionID
      }
    }
    host.close()
    await model.stop()
  }

  @MainActor func test_実行木購読_生きている送り先が無くなると送信できない() async throws {
    // Given
    let http = TestHTTPClient()
    var node = Releash_Client_V1_WorkspaceTreeItem()
    node.node.id = "node"
    node.node.sessionID = "agent"
    node.node.title = "Session"
    node.node.processPresence.value = .live
    var tree = Releash_Client_V1_WorkspaceWorktreeList()
    tree.path = "/repo"
    tree.snapshot.nodes.items = [node]
    var repo = Releash_Client_V1_WorkspaceRepositoryList()
    repo.worktrees.items = [tree]
    http.state.withLock {
      $0.repositories = [repo]
      $0.reviewThreads["/repo"] = .with { $0.items = [testReviewThread("thread")] }
    }
    let model = try await testModel(http)
    let host = await ViewHarness(AnyView(ThreadPanelView(model: model, worktree: "/repo")))
    try await eventuallyView(host) { host.element("thread-check:thread") != nil }
    // When
    try await host.press("thread-check:thread")
    // Then
    try await eventuallyView(host) { host.element("thread-send")?.enabled == true }
    var payload = Releash_Client_V1_StatePayload()
    node.node.processPresence.value = .confirmedAbsent
    tree.snapshot.nodes.items = [node]
    repo.worktrees.items = [tree]
    payload.workspaces.repositories.items = [repo]
    // When
    try http.publish(payload, target: "workspaces")
    // Then
    try await eventuallyView(host) { host.element("thread-send")?.enabled == false }
    // When
    var past = Releash_Client_V1_WorkspacePastAttempt()
    past.node.sessionID = "past"
    past.node.processPresence.value = .live
    node.node.pastAttempts.items = [past]
    tree.snapshot.nodes.items = [node]
    repo.worktrees.items = [tree]
    payload.workspaces.repositories.items = [repo]
    try http.publish(payload, target: "workspaces")
    // Then
    try await eventuallyView(host) { host.element("thread-send")?.enabled == true }
    XCTAssertFalse(http.state.withLock { $0.subscriptions.contains { $0.target == "session-tab" } })
    // When
    payload.workspaces.repositories.items = []
    try http.publish(payload, target: "workspaces")
    // Then
    try await eventuallyView(host) { host.element("thread-send")?.enabled == false }
    host.close()
    await model.stop()
  }
}

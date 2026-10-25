import AppKit
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class ThreadPanelViewTests: XCTestCase {
  @MainActor func test_Thread面_部品とmarkdown本文を表示し状態を切り替える() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.reviewThreads["/repo"] = .with {
        $0.items = [testReviewThread("open"), testReviewThread("resolved", resolved: true)]
      }
    }
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    let host = await ViewHarness(
      RightPanelView(expanded: .constant(true), model: model),
      size: NSSize(width: 360, height: 900))
    // When
    try await host.press("right-panel:threads")
    // Then
    try await eventuallyView(host) { host.element("thread:open") != nil }
    for id in [
      "thread-filter", "thread-check:open", "thread-body:open", "thread-comments:open",
      "thread-reply:open", "thread-resolve:open", "thread-add-general", "thread-send",
    ] {
      XCTAssertNotNil(host.element(id), id)
    }
    XCTAssertNil(host.element("thread:resolved"))
    XCTAssertTrue(host.texts().contains { $0.contains("確認") })
    XCTAssertTrue(host.texts().contains { $0.contains("let value = 1") })
    XCTAssertFalse(try XCTUnwrap(host.element("thread-send")).enabled)
    // When
    try await host.press("thread-check:open")
    try await eventuallyView(host) {
      host.texts().contains("チェックした 1 件を Session に送る")
    }
    try await host.press("thread-reply:open")
    // Then
    try await eventuallyView(host) { host.element("thread-reply-content:open") != nil }
    XCTAssertFalse(try XCTUnwrap(host.element("thread-reply-send:open")).enabled)
    // When
    try await host.press("thread-reply-cancel:open")
    try await host.press("解決済み 1")
    // Then
    try await eventuallyView(host) { host.element("thread:resolved") != nil }
    XCTAssertNil(host.element("thread:open"))
    XCTAssertNotNil(host.element("thread-resolved:resolved"))
    XCTAssertNil(host.element("thread-reply:resolved"))
    XCTAssertNil(host.element("thread-resolve:resolved"))
    // When
    try await host.press("thread-check:resolved")
    // Then
    try await eventuallyView(host) {
      host.texts().contains("チェックした 2 件を Session に送る")
    }
    XCTAssertFalse(try XCTUnwrap(host.element("thread-send")).enabled)
    // When
    try await host.press("thread-add-general")
    // Then
    try await eventuallyView(host) { host.element("thread-general-content") != nil }
    XCTAssertFalse(try XCTUnwrap(host.element("thread-general-create")).enabled)
    XCTAssertNotNil(host.element("thread-general-cancel"))
    try await host.press("thread-general-cancel")
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
        $0.subscriptions.contains { $0.target == "review-threads" && $0.args == ["/repo"] }
      }
    }
    var thread = testReviewThread("created")
    var payload = Releash_Client_V1_StatePayload()
    payload.reviewThreads.items = [thread]
    // When
    try http.publish(payload, target: "review-threads", args: ["/repo"])
    // Then
    try await eventuallyView(host) { host.element("thread:created") != nil }
    // When
    var reply = thread.comments.items[0]
    reply.id = "reply"
    reply.content = "返信本文"
    thread.comments.items.append(reply)
    payload.reviewThreads.items = [thread]
    try http.publish(payload, target: "review-threads", args: ["/repo"])
    // Then
    try await eventuallyView(host) { host.texts().contains("Comment 2件") }
    try await host.press("thread-history:created")
    try await eventuallyView(host) { host.texts().contains("返信本文") }
    // When
    thread.state.value = .resolved
    payload.reviewThreads.items = [thread]
    try http.publish(payload, target: "review-threads", args: ["/repo"])
    // Then
    try await eventuallyView(host) { host.element("thread:created") == nil }
    try await host.press("解決済み 1")
    try await eventuallyView(host) { host.element("thread-resolved:created") != nil }
    // When
    payload.reviewThreads.items = []
    try http.publish(payload, target: "review-threads", args: ["/repo"])
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

  @MainActor func test_Session購読_生きている送り先が無くなると送信できない() async throws {
    // Given
    let http = TestHTTPClient()
    var node = Releash_Client_V1_WorkspaceTreeItem()
    node.node.id = "node"
    node.node.sessionID = "agent"
    node.node.title = "Session"
    var tree = Releash_Client_V1_WorkspaceWorktreeList()
    tree.path = "/repo"
    tree.snapshot.nodes.items = [node]
    var repo = Releash_Client_V1_WorkspaceRepositoryList()
    repo.worktrees.items = [tree]
    http.state.withLock {
      $0.repositories = [repo]
      $0.reviewThreads["/repo"] = .with { $0.items = [testReviewThread("thread")] }
      $0.sessionTabs["agent"] = .with {
        $0.session.id = "agent"
        $0.session.availability = .open
      }
    }
    let model = try await testModel(http)
    let host = await ViewHarness(AnyView(ThreadPanelView(model: model, worktree: "/repo")))
    try await eventuallyView(host) { host.element("thread-check:thread") != nil }
    // When
    try await host.press("thread-check:thread")
    // Then
    try await eventuallyView(host) { host.element("thread-send")?.enabled == true }
    var payload = Releash_Client_V1_StatePayload()
    payload.sessionTab.session.id = "agent"
    payload.sessionTab.session.availability = .paused
    // When
    try http.publish(payload, target: "session-tab", args: ["agent"])
    // Then
    try await eventuallyView(host) { host.element("thread-send")?.enabled == false }
    // When
    model.repositories = []
    // Then
    let subscription = try XCTUnwrap(
      http.state.withLock {
        $0.subscriptions.first { $0.target == "session-tab" && $0.args == ["agent"] }
      })
    try await eventually {
      http.requests("StopStateSubscription").contains {
        (try? Releash_Client_V1_StopStateSubscriptionRequest(serializedBytes: $0.message ?? Data())
          .subscriptionID) == subscription.subscriptionID
      }
    }
    host.close()
    await model.stop()
  }
}

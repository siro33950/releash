import AppKit
import ApplicationServices
import Bonsplit
import SwiftUI
import XCTest

@testable import Releash

final class FileTabPaneTests: XCTestCase {
  @MainActor func pane(_ model: AppModel, value: Releash_Client_V1_FileTabView? = nil) throws
    -> FileTabPane
  {
    try model.filePanes.restore(.empty())
    model.filePanes.openFile("src/file.rs", base: "head")
    return FileTabPane(
      model: model, tabID: try XCTUnwrap(model.filePanes.controller.allTabIds.first),
      path: "src/file.rs", worktree: "/repo", value: value)
  }

  @MainActor func testThreadAnnotationsRetainSideAndServerResolution() throws {
    for side in [Releash_Client_V1_ReviewLineSideDto.Value.deletions, .additions] {
      var thread = Releash_Client_V1_ReviewThreadDto()
      thread.id = "thread"
      thread.target.lineNumber = 2
      thread.target.side.value = side
      thread.target.endLine = 3
      thread.target.endSide.value = side
      thread.canResolve = true
      thread.comments.items = [
        .with {
          $0.author.displayName = "human"
          $0.content = "body"
        }
      ]
      var threads = Releash_Client_V1_FileThreadsSnapshot()
      threads.threads.items = [thread]
      let model = AppModel()
      let base = try pane(model)
      let view = FileTabPane(
        model: model, tabID: base.tabID, path: base.path, worktree: base.worktree, threads: threads)
      for resolved in [false, true] {
        threads.threads.items[0].state.value = resolved ? .resolved : .open
        let snapshot = FileTabPane(
          model: model, tabID: view.tabID, path: view.path, worktree: view.worktree,
          threads: threads)
        let payload = snapshot.bridgePayload(.init(), .init(), colorScheme: .light)
        let annotations = try XCTUnwrap(payload["threads"] as? [[String: Any]])
        XCTAssertEqual(
          annotations[0]["side"] as? String, side == .deletions ? "deletions" : "additions")
        XCTAssertEqual(annotations[0]["line"] as? UInt32, 2)
        XCTAssertEqual(annotations[0]["endLine"] as? UInt32, 3)
        XCTAssertNotNil((payload["accent"] as? String)?.wholeMatch(of: /#[0-9A-F]{6}/))
        XCTAssertEqual(
          annotations[0]["endSide"] as? String, side == .deletions ? "deletions" : "additions")
        XCTAssertEqual(annotations[0]["resolved"] as? Bool, resolved)
        XCTAssertEqual(annotations[0]["canResolve"] as? Bool, true)
        XCTAssertEqual(
          (annotations[0]["comments"] as? [[String: Any]])?.first?["content"] as? String, "body")
      }
    }
  }

  @MainActor func testFileActionsSendTargetAndThreadInputAndReportRPCFailure() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let view = try pane(model)
    view.openInEditor()
    var acceptedReplies = 0
    view.action(["kind": "reply", "id": "thread", "content": "reply body"]) { accepted in
      if accepted { acceptedReplies += 1 }
    }
    view.action(["kind": "resolve", "id": "thread"])
    view.action([
      "kind": "select", "line": NSNumber(value: 2), "endLine": NSNumber(value: 3),
      "side": "deletions", "endSide": "deletions",
    ])
    XCTAssertNil(view.draft.line)
    XCTAssertEqual(view.draft.selection?.endLine, 3)
    view.startThread()
    XCTAssertEqual(view.draft.line, 2)
    XCTAssertEqual(view.draft.side, .deletions)
    view.draft.content = "selected line"
    view.createThread()
    try await eventually {
      http.requests("CreateReviewThread").count == 1
        && http.requests("ResolveReviewThread").count == 1
        && http.requests("AppendReviewComment").count == 1
        && http.requests("OpenInEditor").count == 1 && acceptedReplies == 1
    }
    let open = try Releash_Client_V1_OpenInEditorRequest(
      serializedBytes: XCTUnwrap(http.requests("OpenInEditor").first?.message))
    XCTAssertEqual(open.worktreePath, "/repo")
    XCTAssertEqual(open.file, "src/file.rs")
    XCTAssertEqual(acceptedReplies, 1)
    let reply = try Releash_Client_V1_AppendReviewCommentRequest(
      serializedBytes: XCTUnwrap(http.requests("AppendReviewComment").first?.message))
    XCTAssertEqual(reply.worktreeName, "/repo")
    XCTAssertEqual(reply.threadID, "thread")
    XCTAssertEqual(reply.content, "reply body")
    let resolve = try Releash_Client_V1_ResolveReviewThreadRequest(
      serializedBytes: XCTUnwrap(http.requests("ResolveReviewThread").first?.message))
    XCTAssertEqual(resolve.threadID, "thread")
    XCTAssertEqual(resolve.worktreeName, "/repo")
    XCTAssertEqual(resolve.outcome, "resolved")
    let create = try Releash_Client_V1_CreateReviewThreadRequest(
      serializedBytes: XCTUnwrap(http.requests("CreateReviewThread").first?.message))
    XCTAssertEqual(create.worktreeName, "/repo")
    XCTAssertEqual(create.filePath, "src/file.rs")
    XCTAssertEqual(create.lineNumber, 2)
    XCTAssertEqual(create.endLine, 3)
    XCTAssertEqual(create.content, "selected line")
    XCTAssertEqual(create.side.value, .deletions)
    XCTAssertEqual(create.endSide.value, .deletions)
    XCTAssertNil(view.draft.line)
    view.action([
      "kind": "select", "line": NSNumber(value: 4), "endLine": NSNumber(value: 4),
      "side": "additions", "endSide": "additions",
    ])
    view.startThread()
    XCTAssertEqual(view.draft.side, .additions)
    XCTAssertEqual(view.draft.content, "")
    view.draft.content = "added line"
    view.createThread()
    try await eventually {
      http.requests("CreateReviewThread").count == 2 && view.draft.line == nil
    }
    view.action(["kind": "select"])
    XCTAssertNil(view.draft.selection)
    let added = try Releash_Client_V1_CreateReviewThreadRequest(
      serializedBytes: XCTUnwrap(http.requests("CreateReviewThread").last?.message))
    XCTAssertEqual(added.side.value, .additions)
    XCTAssertEqual(added.lineNumber, 4)
    http.state.withLock { $0.failedMethod = "AppendReviewComment" }
    var replyResult: Bool?
    view.action(["kind": "reply", "id": "thread", "content": "failure"]) { accepted in
      replyResult = accepted
      if accepted { acceptedReplies += 1 }
    }
    try await eventually { model.failure != nil }
    XCTAssertTrue(try XCTUnwrap(model.failure).contains("AppendReviewComment failed"))
    XCTAssertEqual(acceptedReplies, 1)
    XCTAssertEqual(replyResult, false)
    await model.stop()
  }

  @MainActor func test_作成カード_失敗時は本文を保持しキャンセルでは送らない() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let view = try pane(model)
    view.action(["kind": "select", "line": 2, "endLine": 3])
    view.startThread()
    http.state.withLock { $0.failedMethod = "CreateReviewThread" }
    // When
    view.action(["kind": "create", "content": "draft"])
    view.action(["kind": "create", "content": "draft"])
    try await eventually { model.failure != nil && !view.draft.sending }
    // Then
    XCTAssertEqual(http.requests("CreateReviewThread").count, 1)
    XCTAssertEqual(view.draft.content, "draft")
    XCTAssertEqual(view.draft.line, 2)
    XCTAssertNotNil(view.bridgePayload(.init(), .init(), colorScheme: .light)["composer"])
    view.action(["kind": "cancelCreate"])
    XCTAssertNil(view.draft.line)
    XCTAssertNil(view.bridgePayload(.init(), .init(), colorScheme: .light)["composer"])
    XCTAssertEqual(http.requests("CreateReviewThread").count, 1)
    await model.stop()
  }

  @MainActor func test_作成カード_送信中のキャンセルはカードを閉じない() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let view = try pane(model)
    view.action(["kind": "select", "line": 2, "endLine": 3])
    view.startThread()
    view.action(["kind": "create", "content": "draft"])
    XCTAssertTrue(view.draft.sending)
    // When
    view.action(["kind": "cancelCreate"])
    // Then
    XCTAssertEqual(view.draft.line, 2)
    XCTAssertNotNil(view.bridgePayload(.init(), .init(), colorScheme: .light)["composer"])
    try await eventually {
      http.requests("CreateReviewThread").count == 1 && !view.draft.sending
    }
    XCTAssertNil(view.draft.line)
    await model.stop()
  }

  @MainActor func test_作成カード_送信前のキャンセルは作成を送らない() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let view = try pane(model)
    view.action(["kind": "select", "line": 2])
    view.startThread()
    view.draft.content = "draft"
    // When
    view.action(["kind": "cancelCreate"])
    // Then
    XCTAssertNil(view.draft.line)
    XCTAssertNil(view.bridgePayload(.init(), .init(), colorScheme: .light)["composer"])
    XCTAssertTrue(http.requests("CreateReviewThread").isEmpty)
    await model.stop()
  }

  @MainActor func test_作成カード_空白だけの本文は作成を送らない() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let view = try pane(model)
    view.action(["kind": "select", "line": 2])
    view.startThread()
    // When
    view.action(["kind": "create", "content": " \n\t"])
    // Then
    XCTAssertTrue(http.requests("CreateReviewThread").isEmpty)
    XCTAssertFalse(view.draft.sending)
    XCTAssertEqual(view.draft.line, 2)
    await model.stop()
  }

  @MainActor func test_作成カード_比較基準変更後も入力本文を復元用に渡す() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let view = try pane(model)
    view.action(["kind": "select", "line": 2])
    view.startThread()
    let id = view.draft.id.uuidString
    // When
    view.action(["kind": "editCreate", "id": id, "content": "未送信本文"])
    view.setPresentation(base: "branch-base")
    view.subscribe()
    let payload = view.bridgePayload(.init(), .init(), colorScheme: .light)
    // Then
    let composer = try XCTUnwrap(payload["composer"] as? [String: Any])
    XCTAssertEqual(composer["content"] as? String, "未送信本文")
    XCTAssertEqual(composer["id"] as? String, id)
    XCTAssertEqual(payload["base"] as? String, "branch-base")
    XCTAssertTrue(http.requests("CreateReviewThread").isEmpty)
    // When
    view.startThread()
    XCTAssertEqual(view.draft.id.uuidString, id)
    XCTAssertEqual(view.draft.content, "未送信本文")
    view.action(["kind": "select", "line": 5])
    view.startThread()
    // Then
    XCTAssertEqual(view.draft.line, 5)
    XCTAssertEqual(view.draft.content, "未送信本文")
    // When
    view.draft.sending = true
    view.action(["kind": "editCreate", "id": view.draft.id.uuidString, "content": "送信中"])
    // Then
    XCTAssertEqual(view.draft.content, "未送信本文")
    await model.stop()
  }

  @MainActor func test_カードの見出し_Commentと解決者の記録を渡す() throws {
    // Given
    var thread = Releash_Client_V1_ReviewThreadDto()
    thread.target.lineNumber = 1
    thread.resolve.actor.displayName = "resolver"
    thread.resolve.resolvedAt = 200
    thread.comments.items = [
      .with {
        $0.author.displayName = "agent"
        $0.author.kind.value = .agent
        $0.author.model = "model"
        $0.createdAt = 100
        $0.content = "body"
      }
    ]
    let model = AppModel()
    let base = try pane(model)
    let view = FileTabPane(
      model: model, tabID: base.tabID, path: base.path, worktree: base.worktree,
      threads: .with { $0.threads.items = [thread] })
    // When
    let payload = view.bridgePayload(.init(), .init(), colorScheme: .light)
    let annotation = try XCTUnwrap((payload["threads"] as? [[String: Any]])?.first)
    let comment = try XCTUnwrap((annotation["comments"] as? [[String: Any]])?.first)
    // Then
    XCTAssertEqual(annotation["resolvedBy"] as? String, "resolver")
    XCTAssertEqual(annotation["resolvedAt"] as? Double, 200)
    XCTAssertEqual(comment["agent"] as? Bool, true)
    XCTAssertEqual(comment["author"] as? String, "agent")
    XCTAssertEqual(comment["model"] as? String, "model")
    XCTAssertEqual(comment["createdAt"] as? Double, 100)
  }

  @MainActor func testImagesUseOnlyPresentSidesAndBinaryShowsReasonAndSize() throws {
    for (original, modified, initial, expected) in [
      (true, true, "diff", ["変更前の画像", "作業ツリーの画像"]),
      (false, true, "diff", ["作業ツリーの画像"]),
      (true, false, "diff", ["変更前の画像"]),
      (true, true, "file", ["作業ツリーの画像"]),
    ] {
      var image = Releash_Client_V1_ReviewImageDto()
      if original { image.originalURL = "data:image/png;base64,AQID" }
      if modified { image.modifiedURL = "data:image/png;base64,BAUG" }
      var value = Releash_Client_V1_FileTabView()
      value.initialMode = initial
      value.view.image = image
      let view = try pane(AppModel(), value: value)
      XCTAssertEqual(view.imageURLs(image, initialMode: initial).map(\.label), expected)
      XCTAssertEqual(
        view.imageURLs(image, initialMode: initial).map(\.url),
        expected.map { $0 == "変更前の画像" ? image.originalURL : image.modifiedURL })
      view.setPresentation(mode: "file")
      XCTAssertEqual(view.imageURLs(image, initialMode: initial).count, modified ? 1 : 0)
    }
    let view = try pane(AppModel())
    var binary = Releash_Client_V1_ReviewBinaryDto()
    binary.originalSize = 10
    binary.modifiedSize = 20
    XCTAssertEqual(view.binaryDescription(binary), ["画像以外のバイナリは表示できません", "20 bytes"])
    binary.clearModifiedSize()
    XCTAssertEqual(view.binaryDescription(binary), ["画像以外のバイナリは表示できません", "10 bytes"])
    binary.modifiedSize = 0
    XCTAssertEqual(view.binaryDescription(binary).last, "0 bytes")
  }
}

import SwiftUI
import XCTest

@testable import Releash

@MainActor final class CommentImageTests: XCTestCase {
  func test_本文画像_購読データを返して購読を解放する() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let url = try XCTUnwrap(URL(string: "https://example.invalid/image.png"))
    let task = Task { try await model.commentImage(url) }
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains {
          $0.target == "comment-image" && $0.args == [url.absoluteString]
        }
      }
    }
    var payload = Releash_Client_V1_StatePayload()
    payload.commentImage.data = Data([1, 2, 3])
    payload.commentImage.mediaType = "image/png"
    // When
    try http.publish(payload, target: "comment-image", args: [url.absoluteString])
    let image = try await task.value
    // Then
    XCTAssertEqual(image, payload.commentImage)
    try await eventually { http.requests("StopStateSubscription").count == 1 }
    await model.stop()
  }

  func test_本文画像_購読の開始失敗を呼び出し元へ返す() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    http.state.withLock { $0.failedMethod = "StartStateSubscription" }
    // When / Then
    do {
      _ = try await model.commentImage(XCTUnwrap(URL(string: "https://example.invalid/image.png")))
      XCTFail("購読失敗を返していない")
    } catch {
      XCTAssertTrue(model.failureMessage(error).contains("StartStateSubscription failed"))
    }
    await model.stop()
  }

  func test_本文画像_キャンセルすると未完了の購読を解放する() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let url = try XCTUnwrap(URL(string: "https://example.invalid/image.png"))
    let task = Task { try await model.commentImage(url) }
    try await eventually {
      http.state.withLock { $0.subscriptions.contains { $0.target == "comment-image" } }
    }
    // When
    task.cancel()
    // Then
    do {
      _ = try await task.value
      XCTFail("キャンセルした画像を返している")
    } catch { XCTAssertTrue(error is CancellationError) }
    try await eventually { http.requests("StopStateSubscription").count == 1 }
    await model.stop()
  }

  func test_本文画像_配信された取得エラーを呼び出し元へ返す() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let url = try XCTUnwrap(URL(string: "https://example.invalid/image.png"))
    let task = Task { try await model.commentImage(url) }
    try await eventually {
      http.state.withLock { $0.subscriptions.contains { $0.target == "comment-image" } }
    }
    let subscription = try XCTUnwrap(
      http.state.withLock { $0.subscriptions.first { $0.target == "comment-image" } })
    let stream = try XCTUnwrap(http.state.withLock { $0.streams.last })
    var event = Releash_Client_V1_StateSubscriptionEvent()
    event.subscriptionID = subscription.subscriptionID
    event.failure.message = "Image unavailable"
    // When
    try TestHTTPClient.send(event, on: stream)
    // Then
    do {
      _ = try await task.value
      XCTFail("画像の取得失敗を返していない")
    } catch { XCTAssertEqual(error.localizedDescription, "Image unavailable") }
    try await eventually { http.requests("StopStateSubscription").count == 1 }
    await model.stop()
  }
}

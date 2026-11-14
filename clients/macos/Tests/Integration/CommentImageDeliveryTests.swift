import SwiftUI
import WebKit
import XCTest

@testable import Releash

@MainActor final class CommentImageDeliveryTests: XCTestCase {
  func test_本文画像_両画面がconnectで配信された画像を表示する() async throws {
    // Given
    let url = "https://example.invalid/server-owned-image.png"
    let http = TestHTTPClient()
    var thread = testReviewThread("image")
    thread.comments.items[0].content = "![配信画像](\(url))"
    http.state.withLock { $0.reviewThreads["/repo"] = .with { $0.items = [thread] } }
    let model = try await testModel(http)
    let panel = await ViewHarness(
      ThreadPanelView(model: model, worktree: "/repo"), size: NSSize(width: 400, height: 350))
    defer { panel.close() }
    let payload: [String: Any] = [
      "path": "a.rs", "original": "a\n", "modified": "a\n", "patch": "", "mode": "file",
      "base": "head", "style": "split", "full": false, "dark": false, "accent": "#007AFF",
      "threads": [
        [
          "id": "image", "line": 1, "resolved": false,
          "comments": [
            ["author": "human", "content": thread.comments.items[0].content, "createdAt": 0]
          ],
        ]
      ],
    ]
    let file = await ViewHarness(FileWebView(payload: payload, model: model, onAction: { _, _ in }))
    defer { file.close() }
    func findWeb(_ view: NSView) -> WKWebView? {
      if let web = view as? WKWebView { return web }
      return view.subviews.compactMap(findWeb).first
    }
    let web = try XCTUnwrap(findWeb(XCTUnwrap(file.window.contentView)))
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "comment-image" && $0.args == [url] }
      }
    }
    var image = Releash_Client_V1_StatePayload()
    image.commentImage.mediaType = "image/png"
    image.commentImage.data = try XCTUnwrap(
      Data(
        base64Encoded:
          "iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAIAAAD8GO2jAAAAKElEQVR4nO3NsQ0AAAzCMP5/un0CNkuZ41wybXsHAAAAAAAAAAAAxR4yw/wuPL6QkAAAAABJRU5ErkJggg=="
      ))
    // When / Then
    let deadline = ContinuousClock.now + .seconds(5)
    var webImageShown = false
    while ContinuousClock.now < deadline && !webImageShown {
      try http.publish(image, target: "comment-image", args: [url])
      webImageShown =
        (try? await web.callAsyncJavaScript(
          """
          const all = (root) => [...root.querySelectorAll('img'), ...[...root.querySelectorAll('*')].flatMap(e => e.shadowRoot ? all(e.shadowRoot) : [])];
          return all(document).some(e => e.src.startsWith('releash-comment-image:') && e.complete && e.naturalWidth === 32);
          """, arguments: [:], in: nil, contentWorld: .page)) as? Bool == true
      try await Task.sleep(for: .milliseconds(20))
    }
    let diagnostic = try? await web.evaluateJavaScript("document.body.innerHTML")
    XCTAssertTrue(
      webImageShown,
      "FileViewer: \(model.failure ?? "no failure"), DOM=\(diagnostic ?? "unavailable")")
    let view = try XCTUnwrap(panel.window.contentView)
    try await eventually {
      view.displayIfNeeded()
      if let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
        view.cacheDisplay(in: view.bounds, to: bitmap)
        var red = 0
        for y in stride(from: 0, to: bitmap.pixelsHigh, by: 2) {
          for x in stride(from: 0, to: bitmap.pixelsWide, by: 2) {
            guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
            if color.redComponent > 0.7 && color.greenComponent < 0.5 && color.blueComponent < 0.3 {
              red += 1
            }
          }
        }
        return red >= 64
      }
      return false
    }
    await model.stop()
  }
}

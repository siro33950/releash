import AppKit
import ApplicationServices
import Bonsplit
import SwiftUI
import WebKit
import XCTest

@testable import Releash

@MainActor
final class FileTabLifecycleTests: IntegrationTestCase, FileViewerAcceptanceFixture {
  private let path = "acceptance.rs"

  func test_ファイルタブ_差分で自動展開した区間の行を選び直して上端に復元する() async throws {
    // Given
    let defaults = UserDefaults.standard
    let previous = defaults.object(forKey: "diffFull")
    defaults.set(false, forKey: "diffFull")
    defer { defaults.set(previous, forKey: "diffFull") }
    let (http, _, host) = try await filesPanel(fileLayout(["other.rs", path], mode: "diff"))
    host.setContentSize(NSSize(width: 560, height: 480))
    let prefix = (1...80).map { "line-\($0)\n" }.joined()
    var diff = snapshot(prefix + "new\n", mode: "diff")
    diff.fileTab.view.textDiff.original = prefix + "old\n"
    diff.fileTab.patch = "--- a/acceptance.rs\n+++ b/acceptance.rs\n@@ -81 +81 @@\n-old\n+new\n"
    try http.publish(diff, target: "file-tab", args: ["/repo", path, "head"])
    var thread = testReviewThread("prefix")
    thread.target.filePath = path
    thread.target.lineNumber = 5
    thread.target.endLine = 5
    thread.target.side.value = .additions
    var threads = Releash_Client_V1_StatePayload()
    threads.fileThreads.threads.items = [thread]
    try http.publish(threads, target: "file-threads", args: ["/repo", path])
    try await press(host, "right-panel:files")
    let web = try await viewer(host)
    let row = "all('[data-line=\"12\"]').find(visible)"
    let offset =
      "\(row)?.getBoundingClientRect().top - document.querySelector('#viewport').getBoundingClientRect().top"
    try await wait(web, "\(row) && all('[aria-expanded=false]').some(visible)")
    _ = try await js(
      web,
      """
      const viewport = document.querySelector('#viewport');
      viewport.addEventListener('scroll', () => window.didScroll = true, {once: true});
      viewport.scrollTop += \(offset);
      """)
    try await wait(web, "window.didScroll && Math.abs(\(offset)) <= 1")
    // When
    try await press(host, "other.rs")
    try await selectedPath("other.rs", in: host)
    try await press(host, path)
    try await selectedPath(path, in: host)
    // Then
    let restored = try await viewer(host)
    try await wait(restored, "\(row) && Math.abs(\(offset)) <= 1")
  }

  func test_ファイルタブ_最終行の二つ目のThread内の位置を選び直して復元する() async throws {
    // Given
    let (http, _, host) = try await filesPanel(fileLayout(["other.rs", path]))
    host.setContentSize(NSSize(width: 560, height: 480))
    try http.publish(
      snapshot("last", mode: "file"), target: "file-tab", args: ["/repo", path, "head"])
    var threads = Releash_Client_V1_StatePayload()
    threads.fileThreads.threads.items = (1...12).map { index in
      var thread = testReviewThread("thread-\(index)")
      thread.target.filePath = path
      thread.target.lineNumber = 1
      thread.target.endLine = 1
      thread.comments.items[0].content = String(repeating: "段落\n\n", count: 20)
      return thread
    }
    try http.publish(threads, target: "file-threads", args: ["/repo", path])
    try await press(host, "right-panel:files")
    let web = try await viewer(host)
    let cards = "all('[aria-expanded=true]').map(e => e.closest('section'))"
    try await toggleThread(web)
    try await wait(web, "\(cards).length === 1")
    try await toggleThread(web)
    try await wait(web, "\(cards).length === 2")
    let position =
      "\(cards)[1]?.getBoundingClientRect().top - document.querySelector('#viewport').getBoundingClientRect().top"
    _ = try await js(
      web,
      """
      const viewport = document.querySelector('#viewport');
      viewport.addEventListener('scroll', () => window.didScroll = true, {once: true});
      viewport.scrollTop += (\(position)) + 35;
      """)
    try await wait(web, "window.didScroll && Math.abs((\(position)) + 35) <= 1")
    // When
    try await press(host, "other.rs")
    try await selectedPath("other.rs", in: host)
    try await press(host, path)
    try await selectedPath(path, in: host)
    // Then
    let restored = try await viewer(host)
    try await wait(restored, "\(cards).length === 2 && Math.abs((\(position)) + 35) <= 1")
  }

  func test_ファイルタブ_選び直すと画像の表示位置を復元する() async throws {
    // Given
    let (http, _, host) = try await filesPanel(fileLayout(["other.rs", path]))
    host.setContentSize(NSSize(width: 440, height: 480))
    let bitmap = try XCTUnwrap(
      NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: 600, pixelsHigh: 2400, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0))
    var image = Releash_Client_V1_StatePayload()
    let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    image.fileTab.view.image.modifiedURL = "data:image/png;base64," + png.base64EncodedString()
    let args = ["/repo", path, "head"]
    try http.publish(image, target: "file-tab", args: args)
    try await press(host, "ファイル")
    let root = try XCTUnwrap(host.contentView)
    func imageScroll() -> NSScrollView? {
      views(root, NSScrollView.self).first {
        !$0.isHiddenOrHasHiddenAncestor && ($0.documentView?.frame.height ?? 0) >= 2000
      }
    }
    try await eventually { imageScroll() != nil }
    let scroll = try XCTUnwrap(imageScroll())
    let offset = CGPoint(x: 120, y: 180)
    scroll.contentView.scroll(to: offset)
    scroll.reflectScrolledClipView(scroll.contentView)
    try await eventually { scroll.contentView.bounds.origin == offset }
    // When
    try await press(host, "other.rs")
    try await selectedPath("other.rs", in: host)
    try await press(host, path)
    // Then
    try await eventually { imageScroll()?.contentView.bounds.origin == offset }
  }

  func test_ファイルタブ_選び直すと作成前の行選択を捨てる() async throws {
    // Given
    let (http, _, host) = try await filesPanel(fileLayout(["other.rs", path]))
    let value = snapshot("one\ntwo\nthree\nfour", mode: "file")
    try await press(host, "ファイル")
    try await expectStarts(http, in: host, paths: [path])
    try http.publish(value, target: "file-tab", args: ["/repo", path, "head"])
    let web = try await viewer(host)
    try await wait(web, "all('[data-line]').length > 0")
    try await selectLines(web)
    try await eventually { self.findElement(host, name: "Thread作成") != nil }
    // When
    try await press(host, "other.rs")
    try await selectedPath("other.rs", in: host)
    try await press(host, path)
    try await selectedPath(path, in: host)
    // Then
    try http.publish(value, target: "file-tab", args: ["/repo", path, "head"])
    let selected = try await viewer(host)
    try await wait(selected, "all('[data-line]').length > 0")
    XCTAssertNil(findElement(host, name: "Thread作成"))
  }

  func test_ファイルタブ_別の面に移って戻ると選択中の購読だけを再開する() async throws {
    // Given
    let (http, _, host) = try await filesPanel(fileLayout(["hidden.rs", path]))
    try await press(host, "ファイル")
    try await expectStarts(http, in: host, paths: [path])
    let started = subscriptions(http)
    // When
    try await press(host, "Node")
    // Then
    try await eventually {
      Set(started.map(\.subscriptionID)).isSubset(of: self.stoppedSubscriptionIDs(http))
    }
    try await press(host, "ファイル")
    try await expectStarts(http, in: host, paths: [path], after: started.count)
    XCTAssertEqual(subscriptions(http).count - started.count, 2)
  }

  func test_ファイルタブ_比較元を変えると読み込み中を経て新しい内容を表示する() async throws {
    // Given
    let (http, _, host) = try await filesPanel(fileLayout(["hidden.rs", path], mode: "diff"))
    try await press(host, "ファイル")
    try await expectStarts(http, in: host, paths: [path])
    let started = subscriptions(http)
    let original = try XCTUnwrap(started.first { $0.target == "file-tab" })
    // When
    try await press(host, "base と比較")
    // Then
    try await eventually {
      self.stoppedSubscriptionIDs(http).contains(original.subscriptionID)
    }
    try await eventually {
      self.subscriptions(http).dropFirst(started.count).contains {
        $0.target == "file-tab" && $0.args == ["/repo", self.path, "branch-base"]
      }
    }
    try await eventually {
      self.findElement(host, name: "", role: kAXProgressIndicatorRole as String) != nil
    }
    XCTAssertNil(findElement(host, name: "100 bytes"))
    var payload = Releash_Client_V1_StatePayload()
    payload.fileTab.view.binary.modifiedSize = 234
    try http.publish(payload, target: "file-tab", args: ["/repo", path, "branch-base"])
    try await eventually { self.findElement(host, name: "234 bytes") != nil }
    XCTAssertEqual(subscriptions(http).count - started.count, 1)
  }
}

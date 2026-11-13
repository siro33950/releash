import AppKit
import ApplicationServices
import Bonsplit
import ObjectiveC
import SwiftProtobuf
import SwiftUI
import Synchronization
import WebKit
import XCTest

@testable import Releash

@MainActor final class InlineThreadAcceptanceTests: XCTestCase {
  private let path = "acceptance.rs"
  private let firstBody = "先の本文"
  private let secondBody = "後の本文"

  private func thread(resolved: Bool = false, model: Bool = true)
    -> Releash_Client_V1_ReviewThreadDto
  {
    var value = testReviewThread("inline", resolved: resolved)
    value.target.filePath = path
    value.target.lineNumber = 2
    value.target.endLine = 3
    value.target.side.value = .additions
    value.comments.items[0].content = firstBody
    value.comments.items[0].author.displayName = "siro33950"
    value.comments.items[0].author.kind.value = .human
    value.comments.items[0].createdAt = Date().timeIntervalSince1970 - 25 * 60
    var second = value.comments.items[0]
    second.id = "second"
    second.content = secondBody
    second.author.displayName = "Codex"
    second.author.kind.value = .agent
    if model { second.author.model = "acceptance-model" }
    second.createdAt = Date().timeIntervalSince1970 - 3 * 60
    value.comments.items.append(second)
    if resolved {
      value.resolve.actor.displayName = "siro33950"
      value.resolve.resolvedAt = Date().timeIntervalSince1970 - 3600
      value.resolve.outcome = "resolved"
    }
    return value
  }

  private func views<T: NSView>(_ root: NSView, _ type: T.Type) -> [T] {
    (root as? T).map { [$0] } ?? root.subviews.flatMap { views($0, type) }
  }

  private func file(_ threads: [Releash_Client_V1_ReviewThreadDto], mode: String = "diff")
    async throws -> (TestHTTPClient, AppModel, ViewHarness, WKWebView)
  {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    try model.filePanes.restore(.empty())
    model.filePanes.openFile(path, base: "head")
    let id = try XCTUnwrap(model.filePanes.controller.allTabIds.first)
    model.filePanes.setFilePresentation(id, mode: mode, base: nil)
    let host = await ViewHarness(
      FileTabPane(model: model, tabID: id, path: path, worktree: "/repo"))
    addTeardownBlock { @MainActor in
      host.close()
      await model.stop()
    }
    try await eventually {
      http.state.withLock { state in
        ["file-tab", "file-threads"].allSatisfy { target in
          state.subscriptions.contains { $0.target == target }
        }
      }
    }
    var payload = Releash_Client_V1_StatePayload()
    payload.fileTab.initialMode = mode
    payload.fileTab.view.textDiff.path = path
    payload.fileTab.view.textDiff.original = "one\nold\nthree\nfour\n"
    payload.fileTab.view.textDiff.modified = "one\nnew\nthree\nfour\n"
    payload.fileTab.patch =
      "--- a/acceptance.rs\n+++ b/acceptance.rs\n@@ -1,4 +1,4 @@\n one\n-old\n+new\n three\n four\n"
    try http.publish(payload, target: "file-tab")
    try publish(threads, http)
    try await eventually { self.views(host.window.contentView!, WKWebView.self).count == 1 }
    let web = try XCTUnwrap(views(host.window.contentView!, WKWebView.self).first)
    do {
      try await wait(web, "all('[data-line]').length > 0")
    } catch {
      XCTFail("FileViewer: \(model.failure ?? "no model failure"), frame=\(web.frame)")
      throw error
    }
    return (http, model, host, web)
  }

  private func publish(_ threads: [Releash_Client_V1_ReviewThreadDto], _ http: TestHTTPClient)
    throws
  {
    var payload = Releash_Client_V1_StatePayload()
    payload.fileThreads.threads.items = threads
    try http.publish(payload, target: "file-threads")
  }

  // Shadow DOM を含む表示済みの要素を読み、製品の DOM 構造のクラス名には依存しない。
  private let dom = """
    const all = (selector, root = document) => [
      ...root.querySelectorAll(selector),
      ...[...root.querySelectorAll('*')].flatMap(e => e.shadowRoot ? all(selector, e.shadowRoot) : [])
    ];
    const visible = e => e.getClientRects().length > 0 && getComputedStyle(e).visibility !== 'hidden';
    const button = label => all('button').find(e => visible(e) && e.textContent.trim() === label);
    const input = () => all('textarea').find(visible);
    const text = () => all('body, [slot]').filter(visible).map(e => e.innerText).join('\\n');
    const header = (author, body, otherBody, time) => {
      for (const name of all('[slot] *').filter(e => visible(e) && e.textContent.trim() === author)) {
        let comment = name.parentElement;
        while (comment && !comment.innerText.includes(body)) comment = comment.parentElement;
        if (!comment || comment.innerText.includes(otherBody)) continue;
        let heading = name.parentElement;
        while (heading && heading !== comment) {
          if (heading.innerText.includes(time) && !heading.innerText.includes(body)) return {heading, name};
          heading = heading.parentElement;
        }
      }
    };
    const besideName = (icon, name) => {
      const a = icon.getBoundingClientRect(), b = name.getBoundingClientRect();
      return visible(icon) && a.width > 0 && a.height > 0 && a.right <= b.left + 1 && a.top < b.bottom && a.bottom > b.top;
    };
    const humanIcon = () => {
      const h = header('siro33950', '先の本文', '後の本文', '25 分前');
      return h && all('*', h.heading).some(e =>
        e.textContent.trim().toLowerCase() === Array.from(h.name.textContent.trim())[0].toLowerCase()
        && besideName(e, h.name));
    };
    const agentIcon = () => {
      const h = header('Codex', '後の本文', '先の本文', '3 分前');
      return h && all('svg, img, [role=img]', h.heading).some(e =>
        e.textContent.trim().toLowerCase() !== Array.from(h.name.textContent.trim())[0].toLowerCase() && besideName(e, h.name));
    };

    """

  private func js(_ web: WKWebView, _ source: String) async throws -> Any? {
    try await web.callAsyncJavaScript(dom + source, arguments: [:], in: nil, contentWorld: .page)
  }

  private func wait(_ web: WKWebView, _ expression: String) async throws {
    let deadline = ContinuousClock.now + .seconds(5)
    while ContinuousClock.now < deadline {
      if try await js(web, "return Boolean(\(expression));") as? Bool == true { return }
      try await Task.sleep(for: .milliseconds(20))
    }
    let snapshot = try? await js(
      web,
      "return JSON.stringify({url:location.href,body:document.body?.innerHTML,shadows:all('*').filter(e=>e.shadowRoot).map(e=>e.shadowRoot.innerHTML)}).slice(0,4000);"
    )
    XCTFail("表示条件を満たしません: \(expression), DOM=\(snapshot ?? "unavailable")")
    throw NSError(domain: "InlineThreadAcceptance", code: 1)
  }

  private func check(
    _ web: WKWebView, _ expression: String, file: StaticString = #filePath,
    line: UInt = #line
  ) async throws {
    let result = try await js(web, "return Boolean(\(expression));") as? Bool
    XCTAssertEqual(result, true, expression, file: file, line: line)
  }

  private func click(_ web: WKWebView, _ label: String) async throws {
    try await wait(web, "button('\(label)')")
    _ = try await js(web, "button('\(label)').click();")
  }

  private func key(
    _ host: ViewHarness, code: UInt16, characters: String,
    modifiers: NSEvent.ModifierFlags = []
  ) throws {
    let event = try XCTUnwrap(
      NSEvent.keyEvent(
        with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: 0,
        windowNumber: host.window.windowNumber, context: nil, characters: characters,
        charactersIgnoringModifiers: characters, isARepeat: false, keyCode: code))
    host.window.sendEvent(event)
  }

  private func selectLines(_ web: WKWebView, in host: ViewHarness) async throws {
    let screen = try XCTUnwrap(host.window.screen)
    host.window.setFrameOrigin(
      NSPoint(x: screen.visibleFrame.minX + 20, y: screen.visibleFrame.minY + 20))
    let application = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
    XCTAssertEqual(
      AXUIElementSetAttributeValue(application, kAXFrontmostAttribute as CFString, kCFBooleanTrue),
      .success)
    try await eventually {
      host.window.makeKeyAndOrderFront(nil)
      return NSApp.isActive && host.window.isKeyWindow
    }
    let pointer = try XCTUnwrap(CGEvent(source: nil)?.location)
    defer { CGWarpMouseCursorPosition(pointer) }
    var points: [NSPoint] = []
    for line in [2, 3] {
      let result = try await js(
        web,
        "const e = all('[data-column-number=\"\(line)\"]').find(visible); const r = e.getBoundingClientRect(); return [r.x+r.width/2, r.y+r.height/2];"
      )
      let coordinates = try XCTUnwrap(result as? [Double])
      let local = NSPoint(
        x: coordinates[0], y: web.isFlipped ? coordinates[1] : web.bounds.height - coordinates[1])
      points.append(web.convert(local, to: nil))
    }
    var events: [(NSEvent.EventType, NSPoint, CGPoint)] = []
    for (type, point) in [
      (NSEvent.EventType.leftMouseDown, points[0]), (.leftMouseDragged, points[1]),
      (.leftMouseUp, points[1]),
    ] {
      let screenPoint = host.window.convertPoint(toScreen: point)
      XCTAssertTrue(screen.visibleFrame.contains(screenPoint))
      let cursor = CGPoint(
        x: screenPoint.x, y: try XCTUnwrap(NSScreen.screens.first).frame.maxY - screenPoint.y)
      events.append((type, point, cursor))
    }
    var index = 0
    let timer = Timer(timeInterval: 0.1, repeats: true) { timer in
      let done = MainActor.assumeIsolated {
        let (type, point, cursor) = events[index]
        XCTAssertEqual(CGWarpMouseCursorPosition(cursor), .success)
        let event = NSEvent.mouseEvent(
          with: type, location: point, modifierFlags: [],
          timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: host.window.windowNumber,
          context: nil, eventNumber: 1, clickCount: 1, pressure: type == .leftMouseUp ? 0 : 1)
        XCTAssertNotNil(event)
        if let event { NSApp.postEvent(event, atStart: false) }
        index += 1
        return index == events.count
      }
      if done { timer.invalidate() }
    }
    RunLoop.main.add(timer, forMode: .common)
    RunLoop.main.add(timer, forMode: .eventTracking)
    defer { timer.invalidate() }
    try await eventually { index == events.count }
    try await wait(
      web,
      "[2, 3].every(line => all('[data-column-number=\"' + line + '\"][data-selected-line]').some(visible))"
    )
    try await check(
      web,
      "![1, 4].some(line => all('[data-column-number=\"' + line + '\"][data-selected-line]').some(visible))"
    )
  }

  func test_行下のカード_B001_B002_B003_B004_B006_B007() async throws {
    for mode in ["diff", "file"] {
      // Given / When
      let (_, _, _, web) = try await file([thread()], mode: mode)
      try await wait(web, "text().includes('後の本文')")
      // Then
      try await check(web, "text().indexOf('先の本文') < text().indexOf('後の本文')")
      try await check(web, "text().includes('siro33950') && text().includes('25 分前')")
      try await check(
        web,
        "text().includes('Codex') && text().includes('3 分前') && text().includes('acceptance-model')"
      )
      try await check(
        web,
        "humanIcon()"
      )
      try await check(
        web,
        "all('*').some(name => visible(name) && name.textContent === 'Codex' && all('*').some(model => visible(model) && model.textContent === 'acceptance-model' && model.getBoundingClientRect().left >= name.getBoundingClientRect().right && Math.abs(model.getBoundingClientRect().top - name.getBoundingClientRect().top) < 12))"
      )
      try await check(
        web, "!all('[slot] *').some(e => visible(e) && /^(2|3|2[–-]3)$/.test(e.textContent.trim()))"
      )
      try await check(web, "agentIcon()")
      try await check(web, "input()?.placeholder === '返信を書く…' && button('解決にする')")
      try await check(
        web, "!all('[slot]').filter(visible).some(e => /acceptance\\.rs|@@|old/.test(e.innerText))")
      try await check(
        web,
        "all('[data-line=\"3\"]').filter(visible).some(line => all('[slot]').some(card => visible(card) && card.innerText.includes('先の本文') && card.getBoundingClientRect().top >= line.getBoundingClientRect().bottom - 1 && all('[data-line=\"4\"]').filter(visible).some(next => card.getBoundingClientRect().bottom <= next.getBoundingClientRect().top + 1)))"
      )
    }
  }

  func test_本文_B005_Markdownとして整形する() async throws {
    // Given
    var value = thread()
    value.comments.items[0].content = "# 見出し\n\n- 箇条書き\n\n`code` と **強調**"
    // When
    let (_, _, _, web) = try await file([value])
    try await wait(web, "text().includes('箇条書き')")
    // Then
    try await check(web, "all('h1').some(e => visible(e) && e.textContent === '見出し')")
    try await check(web, "all('li').some(e => visible(e) && e.textContent.includes('箇条書き'))")
    try await check(web, "all('code').some(e => visible(e) && e.textContent === 'code')")
    try await check(web, "all('strong, b').some(e => visible(e) && e.textContent === '強調')")
    try await check(
      web, "!text().includes('**強調**') && !text().includes('# 見出し') && !text().includes('`code`')")
  }

  func test_返信_B008_B009_修飾キーで送信と改行を分ける() async throws {
    // Given
    var value = thread()
    let (http, _, host, web) = try await file([value])
    try await wait(web, "input()")
    host.window.makeFirstResponder(web)
    _ = try await js(
      web,
      "input().focus(); input().value = '返信'; input().dispatchEvent(new Event('input', {bubbles:true})); input().setSelectionRange(2,2);"
    )
    // When
    try key(host, code: 36, characters: "\r")
    try await wait(web, "input().value === '返信\\n'")
    // Then
    XCTAssertTrue(http.requests("AppendReviewComment").isEmpty)
    // When
    try key(host, code: 36, characters: "\r", modifiers: .command)
    try await eventually { http.requests("AppendReviewComment").count == 1 }
    let request = try Releash_Client_V1_AppendReviewCommentRequest(
      serializedBytes: XCTUnwrap(http.requests("AppendReviewComment").first?.message))
    XCTAssertEqual(request.threadID, value.id)
    XCTAssertEqual(request.content, "返信\n")
    var reply = value.comments.items[0]
    reply.id = "reply"
    reply.content = request.content
    reply.createdAt = Date().timeIntervalSince1970
    value.comments.items.append(reply)
    try publish([value], http)
    // Then
    try await wait(web, "input().value === '' && text().includes('返信')")
    try await check(web, "text().lastIndexOf('返信') > text().indexOf('後の本文')")
  }

  func test_解決_B010_解決すると一行へ替わる() async throws {
    // Given
    let (http, _, _, web) = try await file([thread()])
    // When
    try await click(web, "解決にする")
    try await eventually { http.requests("ResolveReviewThread").count == 1 }
    let request = try Releash_Client_V1_ResolveReviewThreadRequest(
      serializedBytes: XCTUnwrap(http.requests("ResolveReviewThread").first?.message))
    XCTAssertEqual(request.threadID, "inline")
    var resolved = thread(resolved: true)
    resolved.resolve.resolvedAt = Date().timeIntervalSince1970 - 60
    try publish([resolved], http)
    // Then
    try await wait(web, "button('表示') && !input()")
    try await check(web, "text().includes('siro33950 が解決にしました · 1 分前') && !text().includes('先の本文')")
  }

  func test_解決済み_B011_B012_B013_B014_表示と隠すを切り替える() async throws {
    for mode in ["diff", "file"] {
      // Given / When
      var value = thread(resolved: true)
      value.comments.items[0].content = "**先の本文**"
      let (_, _, _, web) = try await file([value], mode: mode)
      try await wait(web, "button('表示')")
      // Then
      try await check(
        web, "text().includes('siro33950 が解決にしました · 1 時間前') && !text().includes('先の本文')")
      try await check(
        web,
        "all('[data-line=\"3\"]').filter(visible).some(line => button('表示').getBoundingClientRect().top >= line.getBoundingClientRect().bottom - 1)"
      )
      // When
      try await click(web, "表示")
      // Then
      try await wait(web, "button('隠す') && !button('表示') && text().includes('後の本文')")
      try await check(
        web, "text().indexOf('先の本文') < text().indexOf('後の本文') && !input() && !button('解決にする')")
      try await check(
        web,
        "all('strong').some(e => visible(e) && e.textContent === '先の本文') && text().includes('25 分前') && text().includes('3 分前') && text().includes('Codex') && text().includes('acceptance-model')"
      )
      try await check(web, "humanIcon() && agentIcon()")
      // When
      try await click(web, "隠す")
      // Then
      try await wait(
        web, "button('表示') && !button('隠す') && !text().includes('先の本文') && !text().includes('後の本文')"
      )
    }
  }

  func test_作成_B015_B016_B017_行下で作成してキャンセルできる() async throws {
    // Given
    let (http, _, host, web) = try await file([], mode: "file")
    try await selectLines(web, in: host)
    try await eventuallyView(host) { host.element("file-create-thread") != nil }
    // When
    try await host.press("file-create-thread")
    // Then
    XCTAssertNil(host.window.attachedSheet)
    try await wait(
      web,
      "input()?.placeholder === 'コメントを書く…（Markdown）' && button('キャンセル') && button('Thread を作る')")
    try await check(
      web,
      "all('[data-line=\"3\"]').filter(visible).some(e => input().getBoundingClientRect().top >= e.getBoundingClientRect().bottom - 1)"
    )
    try key(host, code: 0, characters: "a")
    try await wait(web, "input().value === 'a'")
    // When
    try await click(web, "キャンセル")
    // Then
    try await wait(web, "!input()")
    XCTAssertTrue(http.requests("CreateReviewThread").isEmpty)
    // When
    try await host.press("file-create-thread")
    try await wait(web, "input()")
    try key(host, code: 0, characters: "a")
    try await click(web, "Thread を作る")
    try await eventually { http.requests("CreateReviewThread").count == 1 }
    let request = try Releash_Client_V1_CreateReviewThreadRequest(
      serializedBytes: XCTUnwrap(http.requests("CreateReviewThread").first?.message))
    XCTAssertEqual(request.filePath, path)
    XCTAssertEqual(request.lineNumber, 2)
    XCTAssertEqual(request.endLine, 3)
    XCTAssertEqual(request.content, "a")
    var created = thread()
    created.comments.items = [created.comments.items[0]]
    created.comments.items[0].content = request.content
    try publish([created], http)
    // Then
    try await wait(
      web, "input()?.placeholder === '返信を書く…' && button('解決にする') && !button('Thread を作る')")
    try await check(
      web,
      "all('[slot]').some(card => visible(card) && card.innerText.includes('a') && card.innerText.includes('siro33950') && card.contains(input()) && all('[data-line=\"3\"]').filter(visible).some(line => card.getBoundingClientRect().top >= line.getBoundingClientRect().bottom - 1) && all('[data-line=\"4\"]').filter(visible).some(line => card.getBoundingClientRect().bottom <= line.getBoundingClientRect().top + 1))"
    )
  }

  func test_agent_B024_model未記録でも見出しと本文が出る() async throws {
    // Given / When
    let (_, _, _, web) = try await file([thread(model: false)])
    try await wait(web, "text().includes('後の本文')")
    // Then
    try await check(
      web,
      "text().includes('Codex') && text().includes('3 分前') && !text().includes('acceptance-model')")
    try await check(web, "agentIcon()")
  }
  func test_本文画像_B018_B019_両方の面に画像が出る() async throws {
    // Given
    let server = try InlineImageServer()
    defer { server.stop() }
    var value = thread()
    value.comments.items[0].content = "![受入画像](\(server.url))"
    // When
    let (_, _, _, web) = try await file([value])
    // Then
    try await wait(
      web,
      "all('img').some(e => visible(e) && e.alt === '受入画像' && e.complete && e.naturalWidth === 1)")
    // When
    let host = try await panel(value)
    // Then
    try await eventuallyView(host) { host.element("受入画像")?.role == "AXImage" }
  }

  func test_本文リンク_B020_B021_両方の面から既定ブラウザへ渡す() async throws {
    // Given
    let original = try XCTUnwrap(
      class_getInstanceMethod(NSWorkspace.self, #selector(NSWorkspace.open(_:))))
    let replacement = try XCTUnwrap(
      class_getInstanceMethod(NSWorkspace.self, #selector(NSWorkspace.inlineAcceptanceOpen(_:))))
    InlineOpenedURLs.values.withLock { $0.removeAll() }
    method_exchangeImplementations(original, replacement)
    defer { method_exchangeImplementations(original, replacement) }
    let url = "https://example.invalid/inline-acceptance"
    var value = thread()
    value.comments.items[0].content = "[受入リンク](\(url))"
    let (_, _, _, web) = try await file([value])
    try await wait(web, "all('a').some(e => visible(e) && e.textContent === '受入リンク')")
    // When
    _ = try await js(web, "all('a').find(e => e.textContent === '受入リンク').click();")
    // Then
    try await eventually { InlineOpenedURLs.values.withLock { $0.map(\.absoluteString) == [url] } }
    let host = try await panel(value)
    try await eventuallyView(host) { host.element("受入リンク") != nil }
    // When
    try await host.press("受入リンク")
    // Then
    try await eventually {
      InlineOpenedURLs.values.withLock { $0.map(\.absoluteString) == [url, url] }
    }
  }

  func test_HTML_B022_B023_両方の面でscriptが操作を起こさない() async throws {
    // Given
    var value = thread()
    value.comments.items[0].content = """
      本文の前
      <script>
      window.inlineExecuted = true;
      for (const kind of ['reply', 'resolve', 'create']) {
        window.webkit.messageHandlers.fileViewer.postMessage({kind, id:'inline', content:'injected', line:2, endLine:3});
      }
      </script>
      <img src="invalid:" onerror="window.inlineExecuted=true;window.webkit.messageHandlers.fileViewer.postMessage({kind:'reply',id:'inline',content:'injected'})">
      本文の後
      """
    // When
    let (http, _, _, web) = try await file([value])
    try await wait(web, "text().includes('本文の後')")
    // Then
    try await check(web, "window.inlineExecuted !== true")
    try await check(web, "input() && !button('表示')")
    for method in ["AppendReviewComment", "ResolveReviewThread", "CreateReviewThread"] {
      XCTAssertTrue(http.requests(method).isEmpty, method)
    }
    // When
    let panelHTTP = TestHTTPClient()
    let host = try await panel(value, http: panelHTTP)
    try await eventuallyView(host) { host.texts().contains { $0.contains("本文の後") } }
    // Then
    XCTAssertNotNil(host.element("thread:inline"))
    XCTAssertNil(host.element("thread-resolved:inline"))
    for method in ["AppendReviewComment", "ResolveReviewThread", "CreateReviewThread"] {
      XCTAssertTrue(panelHTTP.requests(method).isEmpty, method)
    }
    XCTAssertEqual(panelHTTP.state.withLock { $0.reviewThreads["/repo"]?.items }, [value])
  }

  private func panel(
    _ value: Releash_Client_V1_ReviewThreadDto,
    http: TestHTTPClient = TestHTTPClient()
  ) async throws -> ViewHarness {
    http.state.withLock { $0.reviewThreads["/repo"] = .with { $0.items = [value] } }
    let model = try await testModel(http)
    let host = await ViewHarness(ThreadPanelView(model: model, worktree: "/repo"))
    addTeardownBlock { @MainActor in
      host.close()
      await model.stop()
    }
    return host
  }

}

private enum InlineOpenedURLs {
  static let values = Mutex<[URL]>([])
}

extension NSWorkspace {
  @objc fileprivate func inlineAcceptanceOpen(_ url: URL) -> Bool {
    InlineOpenedURLs.values.withLock { $0.append(url) }
    return true
  }
}

private final class InlineImageServer {
  let process = Process()
  let url: String

  init() throws {
    let output = Pipe()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
    process.arguments = [
      "-u", "-c",
      """
      import base64
      from http.server import BaseHTTPRequestHandler, HTTPServer
      class Handler(BaseHTTPRequestHandler):
          def do_GET(self):
              body = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1sAAAAASUVORK5CYII=')
              self.send_response(200)
              self.send_header('Content-Type', 'image/png')
              self.send_header('Content-Length', str(len(body)))
              self.end_headers()
              self.wfile.write(body)
          def log_message(self, *args): pass
      server = HTTPServer(('127.0.0.1', 0), Handler)
      print(server.server_port, flush=True)
      server.serve_forever()
      """,
    ]
    process.standardOutput = output
    process.standardError = FileHandle.nullDevice
    try process.run()
    let port = String(decoding: output.fileHandleForReading.availableData, as: UTF8.self)
      .trimmingCharacters(in: .whitespacesAndNewlines)
    guard UInt16(port) != nil else {
      if process.isRunning { process.terminate() }
      throw NSError(domain: "InlineImageServer", code: 1)
    }
    url = "http://127.0.0.1:\(port)/image.png"
  }

  func stop() {
    if process.isRunning { process.terminate() }
    process.waitUntilExit()
  }
}

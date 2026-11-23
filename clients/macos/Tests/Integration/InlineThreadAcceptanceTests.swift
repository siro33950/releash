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

@MainActor final class InlineThreadAcceptanceTests: IntegrationTestCase, FileViewerAcceptanceFixture
{
  private func hasIcon(_ label: String, _ name: String) -> String {
    """
    all('[aria-label="\(label)"]').filter(visible).some(control =>
      [control, ...control.querySelectorAll('*')].some(e =>
        visible(e) && (e.classList.contains('lucide-\(name)') ||
          [e.getAttribute('src'), e.getAttribute('href'), e.getAttribute('data-icon')].some(value =>
            value && value.split(/[\\/#.]/).includes('\(name)')))))
    """
  }

  private func icons(_ expression: String) -> String {
    """
    (() => {
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
    return \(expression);
    })()
    """
  }

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

  private func file(_ threads: [Releash_Client_V1_ReviewThreadDto], mode: String = "diff")
    async throws -> (TestHTTPClient, AppModel, NSWindow, WKWebView)
  {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    try model.filePanes.restore(.empty())
    model.filePanes.openFile(path, base: "head")
    let id = try XCTUnwrap(model.filePanes.controller.allTabIds.first)
    model.filePanes.setFilePresentation(id, mode: mode, base: nil)
    let host = window(
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
    try await eventually { self.views(host.contentView!, WKWebView.self).count == 1 }
    let web = try XCTUnwrap(views(host.contentView!, WKWebView.self).first)
    web.configuration.preferences.inactiveSchedulingPolicy = .none
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

  private func click(_ web: WKWebView, _ label: String) async throws {
    try await wait(web, "button('\(label)') && !button('\(label)').disabled")
    _ = try await js(web, "button('\(label)').click();")
  }

  private func key(
    _ host: NSWindow, code: UInt16, characters: String,
    modifiers: NSEvent.ModifierFlags = []
  ) throws {
    let event = try XCTUnwrap(
      NSEvent.keyEvent(
        with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: 0,
        windowNumber: host.windowNumber, context: nil, characters: characters,
        charactersIgnoringModifiers: characters, isARepeat: false, keyCode: code))
    host.sendEvent(event)
  }

  func test_ファイルタブ_B008_B017_両状態のThreadを範囲の最終行の下に閉じて表示する() async throws {
    for mode in ["diff", "file"] {
      // Given
      var resolved = thread(resolved: true)
      resolved.id = "resolved"
      resolved.target.lineNumber = 1
      resolved.target.endLine = 2
      resolved.resolve.actor.displayName = "Resolver"
      // When
      let (_, _, _, web) = try await file([thread(), resolved], mode: mode)
      // Then
      for (label, name, time, line) in [
        ("未解決", "siro33950", "3 分前", 3), ("解決済み", "Resolver", "1 時間前", 2),
      ] {
        let card = "all('[slot]').find(e => visible(e) && e.innerText.includes('\(name)'))"
        try await wait(web, card)
        try await check(
          web,
          """
          (() => {
            const card = \(card), rect = card.getBoundingClientRect();
            const above = all('[data-line="\(line)"]').filter(visible);
            const below = all('[data-line="\(line + 1)"]').filter(visible);
            return above.some(e => e.getBoundingClientRect().bottom <= rect.top + 1)
              && below.some(e => e.getBoundingClientRect().top >= rect.bottom - 1)
              && !card.innerText.includes('先の本文') && !card.innerText.includes('後の本文');
          })()
          """)
        if mode == "diff" {
          try await check(web, hasIcon(label, label == "未解決" ? "circle-dot" : "circle-check"))
          try await check(web, hasIcon("Thread を表示", "chevron-right"))
          try await check(
            web,
            """
            (() => {
              const card = \(card);
              const status = card.querySelector('[aria-label="\(label)"]');
              const toggle = card.querySelector('[aria-label="Thread を表示"]');
              const rangeFor = token => {
                const walker = document.createTreeWalker(card, NodeFilter.SHOW_TEXT);
                while (walker.nextNode()) {
                  const node = walker.currentNode, i = node.data.indexOf(token);
                  if (i < 0) continue;
                  const r = document.createRange();
                  r.setStart(node, i); r.setEnd(node, i + token.length);
                  const rect = r.getBoundingClientRect();
                  if (rect.width) return rect;
                }
              };
              const boxes = [status?.getBoundingClientRect(), rangeFor('\(name)'), rangeFor('\(time)'),
                rangeFor('2'), toggle?.getBoundingClientRect()];
              return boxes.every((r,i) => r && r.width > 0 && (!i || boxes[i-1].right <= r.left + 1));
            })()
            """)
        }
      }
    }
  }

  func test_ファイルタブ_B014_長いThread内を最後のCommentまでスクロールできる() async throws {
    // Given
    var value = thread()
    let comment = value.comments.items[0]
    value.comments.items = (1...45).map { index in
      var entry = comment
      entry.id = "entry-\(index)"
      entry.content = "Comment-\(index)"
      entry.createdAt += Double(index)
      return entry
    }
    let (_, _, _, web) = try await file([value])
    // When
    try await toggleThread(web)
    // Then
    try await wait(web, "text().includes('Comment-45')")
    try await check(
      web,
      """
      (() => {
        const card = all('[slot]').find(e => visible(e) && e.innerText.includes('Comment-45'));
        const last = all('*',card).find(e => e.childElementCount === 0 && e.textContent === 'Comment-45');
        for (let e = last?.parentElement; e && card.contains(e); e = e.parentElement) {
          if (!['scroll','auto'].includes(getComputedStyle(e).overflowY) || e.scrollHeight <= e.clientHeight) continue;
          if (last.getBoundingClientRect().bottom <= e.getBoundingClientRect().bottom) continue;
          e.scrollTop += last.getBoundingClientRect().bottom - e.getBoundingClientRect().bottom + 1;
          const bounds = e.getBoundingClientRect(), body = last.getBoundingClientRect();
          return card.getBoundingClientRect().height <= document.querySelector('#viewport').clientHeight * 0.8 + 1
            && body.top >= bounds.top - 1 && body.bottom <= bounds.bottom + 1;
        }
        return false;
      })()
      """)
  }

  func test_行下のカード_B001_B002_B003_B004_B006_B007() async throws {
    for mode in ["diff", "file"] {
      // Given / When
      let (_, _, _, web) = try await file([thread()], mode: mode)
      try await toggleThread(web)
      try await wait(web, "text().includes('後の本文')")
      // Then
      try await check(web, hasIcon("Thread を隠す", "chevron-down"))
      try await check(web, "text().indexOf('先の本文') < text().indexOf('後の本文')")
      try await check(web, "text().includes('siro33950') && text().includes('25 分前')")
      try await check(
        web,
        "text().includes('Codex') && text().includes('3 分前') && text().includes('acceptance-model')"
      )
      try await check(
        web,
        icons("humanIcon()")
      )
      try await check(
        web,
        "all('*').some(name => visible(name) && name.textContent === 'Codex' && all('*').some(model => visible(model) && model.textContent === 'acceptance-model' && model.getBoundingClientRect().left >= name.getBoundingClientRect().right && Math.abs(model.getBoundingClientRect().top - name.getBoundingClientRect().top) < 12))"
      )
      try await check(
        web,
        """
        (() => {
          const article = body => all('*').find(e => visible(e) && e.childElementCount === 0 && e.textContent === body);
          const first = article('先の本文')?.getBoundingClientRect();
          const last = article('後の本文')?.getBoundingClientRect();
          const toggle = all('[aria-label="Thread を隠す"]').find(visible)?.getBoundingClientRect();
          return first && last && toggle && toggle.bottom <= first.top && first.bottom <= last.top
            && last.bottom <= input().getBoundingClientRect().top
            && last.bottom <= button('解決にする').getBoundingClientRect().top;
        })()
        """)
      try await check(web, icons("agentIcon()"))
      try await check(web, "input()?.placeholder === '返信を書く…' && button('解決にする')")
      try await check(
        web,
        "!all('[slot]').filter(visible).some(e => /acceptance\\.rs|@@|old|new/.test(e.innerText))")
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
    try await toggleThread(web)
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
    try await toggleThread(web)
    try await wait(web, "input()")
    host.makeFirstResponder(web)
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
    try await toggleThread(web)
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
    try await wait(web, "all('[aria-label=\"Thread を表示\"]').find(visible) && !input()")
    try await check(web, hasIcon("解決済み", "circle-check"))
    try await check(
      web,
      "all('[aria-label=解決済み]').some(visible) && text().includes('1 分前') && !text().includes('先の本文')"
    )
  }

  func test_解決済み_B011_B012_B013_B014_表示と隠すを切り替える() async throws {
    for mode in ["diff", "file"] {
      // Given / When
      var value = thread(resolved: true)
      value.comments.items[0].content = "**先の本文**"
      let (_, _, _, web) = try await file([value], mode: mode)
      try await wait(web, "all('[aria-label=\"Thread を表示\"]').some(visible)")
      // Then
      try await check(
        web, "text().includes('siro33950') && text().includes('1 時間前') && !text().includes('先の本文')")
      try await check(
        web,
        "all('[data-line=\"3\"]').filter(visible).some(line => all('[aria-label=\"Thread を表示\"]').find(visible).getBoundingClientRect().top >= line.getBoundingClientRect().bottom - 1)"
      )
      // When
      try await toggleThread(web)
      // Then
      try await wait(
        web,
        "all('[aria-label=\"Thread を隠す\"]').find(visible) && !all('[aria-label=\"Thread を表示\"]').find(visible) && text().includes('後の本文')"
      )
      try await check(
        web, "text().indexOf('先の本文') < text().indexOf('後の本文') && !input() && !button('解決にする')")
      try await check(
        web,
        "all('strong').some(e => visible(e) && e.textContent === '先の本文') && text().includes('25 分前') && text().includes('3 分前') && text().includes('Codex') && text().includes('acceptance-model')"
      )
      try await check(web, icons("humanIcon() && agentIcon()"))
      // When
      try await toggleThread(web, label: "Thread を隠す")
      // Then
      try await wait(
        web,
        "all('[aria-label=\"Thread を表示\"]').find(visible) && !all('[aria-label=\"Thread を隠す\"]').find(visible) && !text().includes('先の本文') && !text().includes('後の本文')"
      )
      try await check(web, hasIcon("Thread を表示", "chevron-right"))
    }
  }

  func test_作成_B012_B018_作成カードの高さとキャンセルと作成後の開いたThread() async throws {
    // Given
    let (http, _, host, web) = try await file([], mode: "file")
    try await selectLines(web)
    try await eventually { self.findElement(host, name: "file-create-thread") != nil }
    // When
    try pressCreate(host)
    // Then
    XCTAssertNil(host.attachedSheet)
    try await wait(
      web,
      "input()?.placeholder === 'コメントを書く…（Markdown）' && button('キャンセル') && button('Thread を作る')")
    try await check(
      web,
      "all('[data-line=\"3\"]').filter(visible).some(e => input().getBoundingClientRect().top >= e.getBoundingClientRect().bottom - 1)"
    )
    let originalFrame = host.frame
    host.setContentSize(NSSize(width: originalFrame.width, height: 180))
    try await wait(web, "document.querySelector('#viewport').clientHeight <= 180")
    try await check(
      web,
      "input().closest('section.thread-card').getBoundingClientRect().height <= document.querySelector('#viewport').clientHeight * 0.8 + 1"
    )
    host.setFrame(originalFrame, display: true)
    try key(host, code: 0, characters: "a")
    try await wait(web, "input().value === 'a'")
    // When
    try await click(web, "キャンセル")
    // Then
    try await wait(web, "!input()")
    XCTAssertTrue(http.requests("CreateReviewThread").isEmpty)
    // When
    try pressCreate(host)
    try await wait(web, "input()")
    try key(host, code: 0, characters: "a")
    try await wait(web, "input().value === 'a'")
    var created = thread()
    created.comments.items = [created.comments.items[0]]
    created.comments.items[0].content = "a"
    http.state.withLock { $0.createdReviewThread = created }
    try await click(web, "Thread を作る")
    try await eventually { http.requests("CreateReviewThread").count == 1 }
    let request = try Releash_Client_V1_CreateReviewThreadRequest(
      serializedBytes: XCTUnwrap(http.requests("CreateReviewThread").first?.message))
    XCTAssertEqual(request.filePath, path)
    XCTAssertEqual(request.lineNumber, 2)
    XCTAssertEqual(request.endLine, 3)
    XCTAssertEqual(request.content, "a")
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
    try await toggleThread(web)
    try await wait(web, "text().includes('後の本文')")
    // Then
    try await check(
      web,
      "text().includes('Codex') && text().includes('3 分前') && !text().includes('acceptance-model')")
    try await check(web, icons("agentIcon()"))
  }
  func test_本文画像_B018_B019_両方の面に画像が出る() async throws {
    // Given
    let server = try InlineImageServer()
    defer { server.stop() }
    var value = thread()
    value.comments.items[0].content = "![受入画像](\(server.url))"
    // When
    let (_, _, _, web) = try await file([value])
    try await toggleThread(web)
    // Then
    try await wait(
      web,
      "all('img').some(e => visible(e) && e.alt === '受入画像' && e.complete && e.naturalWidth === 32)")
    // When
    let host = try await panel(value)
    // Then
    try await eventually { self.findElement(host, name: "thread-body:inline") != nil }
    try await eventually {
      let root = host.contentView!
      root.displayIfNeeded()
      guard
        let frame = try? self.elementFrame(
          try XCTUnwrap(self.findElement(host, name: "thread-body:inline")))
      else { return false }
      let rect = root.convert(host.convertFromScreen(frame), from: nil)
      guard let bitmap = root.bitmapImageRepForCachingDisplay(in: rect) else { return false }
      root.cacheDisplay(in: rect, to: bitmap)
      var redPixels = 0
      for y in 0..<bitmap.pixelsHigh {
        for x in 0..<bitmap.pixelsWide {
          guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
          if color.redComponent > 0.7 && color.greenComponent < 0.5 && color.blueComponent < 0.3 {
            redPixels += 1
          }
        }
      }
      return redPixels >= 16 * 16
    }
  }

  func test_本文リンク_B020_ファイルタブから既定ブラウザへ渡す() async throws {
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
    try await toggleThread(web)
    try await wait(web, "all('a').some(e => visible(e) && e.textContent === '受入リンク')")
    // When
    _ = try await js(web, "all('a').find(e => e.textContent === '受入リンク').click();")
    // Then
    try await eventually { InlineOpenedURLs.values.withLock { $0.map(\.absoluteString) == [url] } }
  }

  func test_HTML_B022_B023_両方の面でscriptが操作を起こさない() async throws {
    // Given
    var value = thread()
    value.comments.items[0].content = """
      本文の前
      <script>
      window.inlineExecuted = true;
      const roots = [document];
      for (const root of roots) {
        for (const element of root.querySelectorAll('*')) {
          if (element.shadowRoot) roots.push(element.shadowRoot);
          if (element.tagName === 'TEXTAREA' && element.placeholder === '返信を書く…') {
            element.value = 'injected';
            element.dispatchEvent(new Event('input', {bubbles:true}));
            element.dispatchEvent(new KeyboardEvent('keydown', {key:'Enter', metaKey:true, bubbles:true}));
          }
        }
      }
      </script>
      <img src="invalid:" onerror="window.inlineExecuted=true">
      本文の後
      """
    // When
    let (http, _, _, web) = try await file([value])
    try await toggleThread(web)
    try await wait(web, "text().includes('本文の後')")
    try await Task.sleep(for: .seconds(1))
    // Then
    try await check(web, "window.inlineExecuted !== true")
    try await check(web, "input() && !all('[aria-label=\"Thread を表示\"]').find(visible)")
    for method in ["AppendReviewComment", "ResolveReviewThread", "CreateReviewThread"] {
      XCTAssertTrue(http.requests(method).isEmpty, method)
    }
    // When
    let panelHTTP = TestHTTPClient()
    let host = try await panel(value, http: panelHTTP)
    try await eventually { self.findElement(host, name: "本文の後", exact: false) != nil }
    try await Task.sleep(for: .seconds(1))
    // Then
    XCTAssertNotNil(findElement(host, name: "thread:inline"))
    XCTAssertNil(findElement(host, name: "thread-resolved:inline"))
    for method in ["AppendReviewComment", "ResolveReviewThread", "CreateReviewThread"] {
      XCTAssertTrue(panelHTTP.requests(method).isEmpty, method)
    }
    XCTAssertEqual(panelHTTP.state.withLock { $0.reviewThreads["/repo"]?.items }, [value])
  }

  private func elementFrame(_ body: AXUIElement) throws -> NSRect {
    var rawPosition: CFTypeRef?
    var rawSize: CFTypeRef?
    XCTAssertEqual(
      AXUIElementCopyAttributeValue(body, kAXPositionAttribute as CFString, &rawPosition), .success)
    XCTAssertEqual(
      AXUIElementCopyAttributeValue(body, kAXSizeAttribute as CFString, &rawSize), .success)
    let position = try XCTUnwrap(rawPosition) as! AXValue
    let size = try XCTUnwrap(rawSize) as! AXValue
    var origin = CGPoint.zero
    var dimensions = CGSize.zero
    XCTAssertTrue(AXValueGetValue(position, .cgPoint, &origin))
    XCTAssertTrue(AXValueGetValue(size, .cgSize, &dimensions))
    return NSRect(
      x: origin.x,
      y: try XCTUnwrap(NSScreen.screens.first).frame.maxY - origin.y - dimensions.height,
      width: dimensions.width, height: dimensions.height)
  }

  private func panel(
    _ value: Releash_Client_V1_ReviewThreadDto,
    http: TestHTTPClient = TestHTTPClient()
  ) async throws -> NSWindow {
    http.state.withLock { $0.reviewThreads["/repo"] = .with { $0.items = [value] } }
    let model = try await testModel(http)
    let host = window(ThreadPanelView(model: model, worktree: "/repo"))
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
              body = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAIAAAD8GO2jAAAAKElEQVR4nO3NsQ0AAAzCMP5/un0CNkuZ41wybXsHAAAAAAAAAAAAxR4yw/wuPL6QkAAAAABJRU5ErkJggg==')
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

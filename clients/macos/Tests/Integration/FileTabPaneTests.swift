import AppKit
import ApplicationServices
import Bonsplit
import SwiftUI
import WebKit
import XCTest

@testable import Releash

final class FileTabPaneTests: IntegrationTestCase {
  @MainActor func pane(_ model: AppModel, value: Releash_Client_V1_FileTabView? = nil) throws
    -> FileTabPane
  {
    try model.filePanes.restore(.empty())
    model.filePanes.openFile("src/file.rs", base: "head")
    return FileTabPane(
      model: model, tabID: try XCTUnwrap(model.filePanes.controller.allTabIds.first),
      path: "src/file.rs", worktree: "/repo", value: value)
  }
  @MainActor func testToolbarShowsModeSpecificItemsAndFitsDefaultPaneWidthWithLongPath()
    async throws
  {
    let common = ["file-path", "file-mode", "file-open-editor"]
    let diffOnly = ["file-base", "file-style", "file-full"]
    for mode in ["file", "diff"] {
      let model = AppModel()
      let original = try pane(model)
      original.setPresentation(mode: mode)
      let path = String(repeating: "long-directory/", count: 30) + "file.rs"
      let value = Releash_Client_V1_FileTabView()
      let view = FileTabPane(
        model: model, tabID: original.tabID, path: path, worktree: "/repo", value: value)
      let width =
        UIStyle.windowWidth - 2 * UIStyle.windowPadding - 2 * UIStyle.sidebarWidth - 2
        * UIStyle.dividerWidth
      let host = NSHostingView(rootView: view.toolbar)
      host.sizingOptions = []
      let window = NSWindow(
        contentRect: NSRect(x: 0, y: 0, width: width, height: 44), styleMask: [.borderless],
        backing: .buffered, defer: false)
      window.isReleasedWhenClosed = false
      window.contentView = host
      window.orderFront(nil)
      defer { window.close() }
      host.layoutSubtreeIfNeeded()
      window.displayIfNeeded()
      let requestResult = await Task.detached {
        let application = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
        var windows: CFTypeRef?
        return AXUIElementCopyAttributeValue(application, kAXWindowsAttribute as CFString, &windows)
          .rawValue
      }.value
      XCTAssertEqual(requestResult, AXError.success.rawValue)
      var elements: [String: AnyObject] = [:]
      var visited = Set<ObjectIdentifier>()
      var scrollAreas = 0
      func collect(_ value: Any) {
        let element = value as AnyObject
        guard visited.insert(ObjectIdentifier(element)).inserted else { return }
        if let id = element.accessibilityIdentifier?() { elements[id] = element }
        if element.accessibilityRole?() == .scrollArea { scrollAreas += 1 }
        for child in element.accessibilityChildren?() ?? [] { collect(child) }
      }
      try await eventually {
        elements.removeAll()
        visited.removeAll()
        scrollAreas = 0
        collect(host)
        return common.allSatisfy { elements[$0] != nil }
      }
      XCTAssertEqual(scrollAreas, 0)
      XCTAssertEqual(
        Set(elements.keys).intersection(Set(common + diffOnly)),
        Set(common + (mode == "diff" ? diffOnly : [])))
      XCTAssertLessThanOrEqual(host.bounds.width, width)
      let visible = window.convertToScreen(host.bounds).insetBy(dx: -1, dy: -1)
      for id in common + (mode == "diff" ? diffOnly : []) {
        let element = try XCTUnwrap(elements[id], id)
        let frame = try XCTUnwrap(element.accessibilityFrame?())
        XCTAssertGreaterThan(frame.width, 0, id)
        XCTAssertGreaterThan(frame.height, 0, id)
        XCTAssertTrue(visible.contains(frame), "\(mode): \(id) \(frame) outside \(visible)")
      }
      let fullPathWidth = NSAttributedString(
        string: path, attributes: [.font: NSFont.systemFont(ofSize: NSFont.systemFontSize)]
      ).size().width
      let pathElement = try XCTUnwrap(elements["file-path"])
      XCTAssertLessThan(try XCTUnwrap(pathElement.accessibilityFrame?()).width, fullPathWidth)
    }
  }

  @MainActor func testDisplaySettingsReachAllTabsAndSurviveNewAppState() async throws {
    let defaults = UserDefaults.standard
    let keys = ["diffStyle", "diffFull", "diffBase"]
    let saved = keys.map { defaults.object(forKey: $0) }
    defer { for (key, value) in zip(keys, saved) { defaults.set(value, forKey: key) } }
    for key in keys { defaults.removeObject(forKey: key) }
    let model = AppModel()
    let first = try pane(model)
    model.openFile("default.rs")
    XCTAssertEqual(
      model.filePanes.tab(try XCTUnwrap(model.filePanes.controller.allTabIds.last))?.fileBase,
      "head")
    defaults.set("branch-base", forKey: "diffBase")
    model.openFile("base.rs")
    XCTAssertEqual(
      model.filePanes.tab(try XCTUnwrap(model.filePanes.controller.allTabIds.last))?.fileBase,
      "branch-base"
    )
    let second = try pane(AppModel())
    var text = Releash_Client_V1_ReviewTextDiffDto()
    text.original = "old\n"
    text.modified = "new\n"
    var value = Releash_Client_V1_FileTabView()
    value.patch = "server patch"
    first.settings.style = "unified"
    first.settings.full = true
    var payload = second.bridgePayload(text, value, colorScheme: .dark)
    XCTAssertEqual(payload["style"] as? String, "unified")
    XCTAssertEqual(payload["full"] as? Bool, true)
    XCTAssertEqual(payload["dark"] as? Bool, true)
    let restarted = AppModel()
    let reopened = try pane(restarted)
    payload = reopened.bridgePayload(text, value, colorScheme: .light)
    XCTAssertEqual(payload["style"] as? String, "unified")
    XCTAssertEqual(payload["full"] as? Bool, true)
    XCTAssertEqual(payload["dark"] as? Bool, false)
    restarted.openFile("new.rs")
    XCTAssertEqual(
      restarted.filePanes.tab(try XCTUnwrap(restarted.filePanes.controller.allTabIds.last))?
        .fileBase,
      "branch-base")
    second.settings.style = "split"
    second.settings.full = false
    var liveStyle = ""
    var liveFull = true
    let host = NSHostingView(
      rootView: SettingsProbe { style, full in
        liveStyle = style
        liveFull = full
      })
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 100, height: 100), styleMask: [.borderless],
      backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.contentView = host
    defer { window.close() }
    host.layoutSubtreeIfNeeded()
    try await eventually { liveStyle == "split" && !liveFull }
    first.settings.style = "unified"
    first.settings.full = true
    try await eventually { liveStyle == "unified" && liveFull }
    second.settings.style = "split"
    second.settings.full = false
    try await eventually { liveStyle == "split" && !liveFull }
  }
}
private struct SettingsProbe: View {
  var settings = FileDisplaySettings()
  var receive: (String, Bool) -> Void
  var body: some View {
    Text(settings.style)
      .onChange(of: settings.style, initial: true) { _, value in receive(value, settings.full) }
      .onChange(of: settings.full, initial: true) { _, value in receive(settings.style, value) }
  }
}

@MainActor
final class FileTabSubscriptionAcceptanceTests: IntegrationTestCase, FileViewerAcceptanceFixture {
  private let path = "acceptance.rs"

  func test_ファイルタブ_B001_Workspaceを選び直すと選択中のファイルだけを購読する() async throws {
    // Given
    let (http, model, host) = try await filesPanel(fileLayout(["hidden.rs", path]))
    model.selectWorktree("/other")
    try await press(host, "right-panel:files")
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "directory" && $0.args.first == "/other" }
      }
    }
    let count = subscriptions(http).count
    // When
    model.selectWorktree("/repo")
    // Then
    try await expectStarts(http, in: host, paths: [path], after: count)
  }

  func test_ファイルタブ_B003_分割した各paneで選択中のファイルだけを購読する() async throws {
    // Given
    let layout = PaneLayout.split(
      id: UUID().uuidString, axis: .horizontal, ratio: 0.5,
      first: fileLayout(["left-hidden.rs", "left.rs"]),
      second: fileLayout(["right-hidden.rs", "right.rs"]))
    let (http, _, host) = try await filesPanel(layout)
    // When
    try await press(host, "right-panel:files")
    // Then
    try await expectStarts(http, in: host, paths: ["left.rs", "right.rs"])
  }

  func test_ファイルタブ_B002_同じpaneで選択を移すと購読対象が切り替わる() async throws {
    // Given
    let (http, _, host) = try await filesPanel(fileLayout(["other.rs", path]))
    try await press(host, "right-panel:files")
    try await expectStarts(http, in: host, paths: [path])
    let started = subscriptions(http)
    // When
    try await press(host, "other.rs")
    // Then
    try await expectStarts(http, in: host, paths: ["other.rs"], after: started.count)
    try await eventually {
      Set(started.map(\.subscriptionID)).isSubset(of: self.stoppedSubscriptionIDs(http))
    }
  }

  func test_ファイルタブ_B004_選択したファイルの最新の内容と差分とThreadを表示する() async throws {
    // Given
    let (http, _, host) = try await filesPanel(fileLayout([path, "other.rs"], mode: "diff"))
    let args = ["/repo", path, "head"]
    try http.publish(
      snapshot("one\nold\nthree\nfour\n", mode: "diff"), target: "file-tab", args: args)
    try await press(host, "right-panel:files")
    try await selectedPath("other.rs", in: host)
    var updated = snapshot("one\nupdated-line\nthree\nfour\n", mode: "diff")
    updated.fileTab.patch =
      "--- a/acceptance.rs\n+++ b/acceptance.rs\n@@ -1,4 +1,4 @@\n one\n-old\n+updated-line\n three\n four\n"
    try http.publish(updated, target: "file-tab", args: args)
    var value = testReviewThread("changed")
    value.target.filePath = path
    value.target.lineNumber = 2
    value.target.endLine = 2
    value.target.side.value = .additions
    value.comments.items[0].content = "先の本文"
    var threads = Releash_Client_V1_StatePayload()
    threads.fileThreads.threads.items = [value]
    try http.publish(threads, target: "file-threads", args: ["/repo", path])
    // When
    try await press(host, path)
    try await selectedPath(path, in: host)
    // Then
    let web = try await viewer(host)
    try await wait(
      web,
      "all('[data-line]').some(e => visible(e) && e.textContent.includes('updated-line')) && all('[data-line]').some(e => visible(e) && e.textContent.includes('old')) && text().includes('Reviewer')"
    )
  }

  func test_ファイルタブ_B005_選び直したファイルでスクロール位置とThreadの下書きを保つ() async throws {
    // Given
    let (http, _, host) = try await filesPanel(fileLayout(["other.rs", path]))
    try await press(host, "right-panel:files")
    try await eventually {
      self.subscriptions(http).contains { $0.target == "file-tab" && $0.args.contains(self.path) }
    }
    let payload = snapshot((1...180).map { "line-\($0)" }.joined(separator: "\n"), mode: "file")
    let args = ["/repo", path, "head"]
    try http.publish(payload, target: "file-tab", args: args)
    var thread = testReviewThread("draft-reply")
    thread.target.filePath = path
    thread.target.lineNumber = 5
    var threads = Releash_Client_V1_StatePayload()
    threads.fileThreads.threads.items = [thread]
    try http.publish(threads, target: "file-threads", args: ["/repo", path])
    let web = try await viewer(host)
    try await wait(web, "all('[data-line]').length > 0")
    try await toggleThread(web)
    try await selectLines(web)
    try await eventually { self.findElement(host, name: "file-create-thread") != nil }
    try pressCreate(host)
    let creation = "all('textarea').find(e => visible(e) && e.placeholder.startsWith('コメントを書く'))"
    let reply = "all('textarea').find(e => visible(e) && e.placeholder === '返信を書く…')"
    try await wait(web, "\(creation) && \(reply)")
    _ = try await js(
      web,
      """
      for (const [field, value] of [[\(creation), '未送信'], [\(reply), '返信の下書き']]) {
        field.value = value;
        field.dispatchEvent(new Event('input', { bubbles: true }));
      }
      const viewport = document.querySelector('#viewport');
      viewport.addEventListener('scroll', () => window.didScroll = true, { once: true });
      viewport.scrollTop = 85;
      """
    )
    try await wait(web, "window.didScroll")
    let offset = try await js(web, "return document.querySelector('#viewport').scrollTop;") as? Int
    XCTAssertGreaterThan(try XCTUnwrap(offset), 0)
    // When
    try await press(host, "other.rs")
    try await selectedPath("other.rs", in: host)
    try await press(host, path)
    try await selectedPath(path, in: host)
    try http.publish(payload, target: "file-tab", args: args)
    // Then
    let selected = try await viewer(host)
    try await wait(
      selected,
      "\(creation)?.value === '未送信' && \(reply)?.value === '返信の下書き' && document.querySelector('#viewport').scrollTop === \(try XCTUnwrap(offset))"
    )
  }

  func test_ファイルタブ_B015_B016_開いたThreadの下と途中の位置を復元する() async throws {
    for inside in [false, true] {
      // Given
      let (http, _, host) = try await filesPanel(fileLayout(["other.rs", path]))
      let payload = snapshot((1...180).map { "line-\($0)" }.joined(separator: "\n"), mode: "file")
      let args = ["/repo", path, "head"]
      try http.publish(payload, target: "file-tab", args: args)
      var thread = testReviewThread("scroll")
      thread.target.filePath = path
      thread.target.lineNumber = 5
      thread.target.endLine = 5
      var threads = Releash_Client_V1_StatePayload()
      threads.fileThreads.threads.items = [thread]
      try http.publish(threads, target: "file-threads", args: ["/repo", path])
      try await press(host, "right-panel:files")
      let web = try await viewer(host)
      try await toggleThread(web)
      try await wait(web, "input()")
      let anchor =
        inside ? "input()?.closest('[slot]')" : "all('[data-line=\\\"12\\\"]').find(visible)"
      let distance = inside ? 35 : 0
      let position =
        "\(anchor)?.getBoundingClientRect().top - document.querySelector('#viewport').getBoundingClientRect().top"
      try await wait(web, anchor)
      _ = try await js(
        web,
        """
        const viewport = document.querySelector('#viewport');
        viewport.addEventListener('scroll', () => window.didScroll = true, {once:true});
        viewport.scrollTop += \(position) + \(distance);
        """)
      try await wait(web, "window.didScroll && Math.abs((\(position)) + \(distance)) <= 1")
      // When
      try await press(host, "other.rs")
      try await selectedPath("other.rs", in: host)
      try await press(host, path)
      try await selectedPath(path, in: host)
      // Then
      let restored = try await viewer(host)
      try await wait(restored, "input() && Math.abs((\(position)) + \(distance)) <= 1")
    }
  }

  private func manuallyExpandedDiff() async throws -> (NSWindow, WKWebView) {
    disableFullDiff()
    let (http, _, host) = try await filesPanel(fileLayout(["other.rs", path], mode: "diff"))
    host.setContentSize(NSSize(width: 560, height: 480))
    let prefix = (1...80).map { "line-\($0)\n" }.joined()
    var diff = snapshot(prefix + "new\n", mode: "diff")
    diff.fileTab.view.textDiff.original = prefix + "old\n"
    diff.fileTab.patch = "--- a/acceptance.rs\n+++ b/acceptance.rs\n@@ -81 +81 @@\n-old\n+new\n"
    try http.publish(diff, target: "file-tab", args: ["/repo", path, "head"])
    try await press(host, "right-panel:files")
    let web = try await viewer(host)
    let expand = "all('[data-expand-button]').find(visible)"
    try await wait(web, "\(expand) && !all('[data-line=\"3\"]').some(visible)")
    _ = try await js(web, "\(expand).click();")
    try await wait(web, "all('[data-line=\"3\"]').some(visible)")
    return (host, web)
  }

  private func reselectFile(in host: NSWindow) async throws -> WKWebView {
    try await press(host, "other.rs")
    try await selectedPath("other.rs", in: host)
    try await press(host, path)
    try await selectedPath(path, in: host)
    let web = try await viewer(host)
    try await wait(web, "document.querySelector('#viewport')")
    return web
  }

  func test_ファイルタブ_B020_手動で展開した行を選び直したときに上端へ戻す() async throws {
    // Given
    let (host, web) = try await manuallyExpandedDiff()
    let offset =
      "all('[data-line=\"12\"]').find(visible)?.getBoundingClientRect().top - document.querySelector('#viewport').getBoundingClientRect().top"
    try await wait(web, "Number.isFinite(\(offset))")
    _ = try await js(
      web,
      """
      const viewport = document.querySelector('#viewport');
      viewport.addEventListener('scroll', () => window.didScroll = true, {once: true});
      viewport.scrollTop += \(offset);
      """)
    try await wait(web, "window.didScroll && Math.abs(\(offset)) <= 1")
    // When
    let restored = try await reselectFile(in: host)
    // Then
    try await wait(restored, "Math.abs(\(offset)) <= 1")
  }

  func test_ファイルタブ_B021_手動で展開した行の下に作成カードと下書きが戻る() async throws {
    // Given
    let (host, web) = try await manuallyExpandedDiff()
    try await selectLines(web)
    try await eventually { self.findElement(host, name: "file-create-thread") != nil }
    try pressCreate(host)
    try await wait(web, "input()")
    _ = try await js(
      web,
      """
      input().value = '書きかけ';
      input().dispatchEvent(new Event('input', {bubbles: true}));
      """)
    // When
    let restored = try await reselectFile(in: host)
    // Then
    try await wait(
      restored,
      """
      (() => {
        const field = input();
        if (field?.value !== '書きかけ') return false;
        const card = field.closest('[slot]').getBoundingClientRect();
        return all('[data-line="3"]').filter(visible).some(e => e.getBoundingClientRect().bottom <= card.top + 1)
          && all('[data-line="4"]').filter(visible).some(e => e.getBoundingClientRect().top >= card.bottom - 1);
      })()
      """)
  }

  func test_ファイルタブ_B006_再購読の内容が届くまで読み込み中を表示する() async throws {
    // Given
    let (http, _, host) = try await filesPanel(fileLayout(["other.rs", path]))
    let threadArgs = ["/repo", path]
    var threads = Releash_Client_V1_StatePayload()
    threads.fileThreads = .init()
    try await press(host, "right-panel:files")
    try await expectStarts(http, in: host, paths: [path])
    try await press(host, "other.rs")
    try await selectedPath("other.rs", in: host)
    http.state.withLock {
      $0.deferredInitialTargets.formUnion(["file-tab", "file-threads"])
    }
    let count = subscriptions(http).count
    // When
    try await press(host, path)
    try await selectedPath(path, in: host)
    try await waitForFileSubscriptions(http, paths: [path], after: count)
    // Then
    try await eventually {
      self.findElement(host, name: "", role: kAXProgressIndicatorRole as String) != nil
        && self.findElement(host, name: "100 bytes") == nil
    }
    var content = Releash_Client_V1_StatePayload()
    content.fileTab.view.binary.modifiedSize = 234
    try http.publish(content, target: "file-tab", args: ["/repo", path, "head"])
    let deadline = ContinuousClock.now + .seconds(1)
    repeat {
      XCTAssertNotNil(findElement(host, name: "", role: kAXProgressIndicatorRole as String))
      XCTAssertNil(findElement(host, name: "100 bytes"))
      XCTAssertNil(findElement(host, name: "234 bytes"))
      try await Task.sleep(for: .milliseconds(20))
    } while ContinuousClock.now < deadline
    try http.publish(threads, target: "file-threads", args: threadArgs)
    try await eventually { self.findElement(host, name: "234 bytes") != nil }
  }

  func test_ファイルタブ_B007_選択を外れたタブの中身を破棄する() async throws {
    // Given
    let (http, _, host) = try await filesPanel(fileLayout(["other.rs", path]))
    try http.publish(
      snapshot("discarded-line", mode: "file"), target: "file-tab", args: ["/repo", path, "head"])
    try await press(host, "right-panel:files")
    let containsLine = "all('[data-line]').some(e => e.textContent.includes('discarded-line'))"
    weak var original: WKWebView?
    do {
      let web = try await viewer(host)
      try await wait(web, containsLine)
      original = web
    }
    // When
    try await press(host, "other.rs")
    try await selectedPath("other.rs", in: host)
    // Then
    let deadline = ContinuousClock.now + .seconds(5)
    while let web = original {
      if try await js(web, "return \(containsLine);") as? Bool == false { return }
      guard ContinuousClock.now < deadline else {
        return XCTFail("選択を外れたタブに discarded-line が残っています")
      }
      try await Task.sleep(for: .milliseconds(20))
    }
  }
}

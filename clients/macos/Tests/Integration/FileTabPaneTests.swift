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

  private func press(_ host: NSWindow, _ name: String) async throws {
    try await eventually { self.findElement(host, name: name) != nil }
    let target = try XCTUnwrap(findElement(host, name: name))
    XCTAssertEqual(AXUIElementPerformAction(target, kAXPressAction as CFString), .success)
  }

  private func selectedPath(_ name: String, in host: NSWindow) async throws {
    try await eventually {
      guard let element = self.findElement(host, name: "file-path") else { return false }
      var value: CFTypeRef?
      return AXUIElementCopyAttributeValue(element, kAXValueAttribute as CFString, &value)
        == .success && value as? String == name
    }
  }

  private func fileLayout(_ names: [String], mode: String = "file") -> PaneLayout {
    let tabs = names.map { PaneTab(kind: .file, filePath: $0, fileMode: mode, fileBase: "head") }
    return .pane(id: UUID().uuidString, tabs: tabs, active: tabs.last?.id)
  }

  private func filesPanel(_ layout: PaneLayout) async throws -> (
    TestHTTPClient, AppModel, NSWindow
  ) {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    try model.filePanes.restore(layout)
    model.saveFilePaneLayout()
    let host = window(RightPanelView(expanded: .constant(true), model: model))
    addTeardownBlock { @MainActor in
      host.close()
      await model.stop()
    }
    return (http, model, host)
  }

  private func subscriptions(_ http: TestHTTPClient)
    -> [Releash_Client_V1_StartStateSubscriptionRequest]
  {
    http.state.withLock { state in
      state.subscriptions.filter {
        ["file-tab", "file-threads"].contains($0.target) && $0.args.first == "/repo"
      }
    }
  }

  private func expectStarts(_ http: TestHTTPClient, paths: [String], after count: Int = 0)
    async throws
  {
    let expected = Set(paths.flatMap { ["file-tab/" + $0, "file-threads/" + $0] })
    func received() -> Set<String> {
      Set(
        subscriptions(http).dropFirst(count).map {
          $0.target + "/" + ($0.args.dropFirst().first ?? "")
        })
    }
    try await eventually { expected.isSubset(of: received()) }
    XCTAssertEqual(received(), expected)
  }

  func test_ファイルタブ_B001_Workspaceを選び直すと選択中のファイルだけを購読する() async throws {
    // Given
    let (http, model, host) = try await filesPanel(fileLayout(["hidden.rs", path]))
    model.selectWorktree("/other")
    try await press(host, "ファイル")
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "directory" && $0.args.first == "/other" }
      }
    }
    let count = subscriptions(http).count
    // When
    model.selectWorktree("/repo")
    // Then
    try await expectStarts(http, paths: [path], after: count)
  }

  func test_ファイルタブ_B003_分割した各paneで選択中のファイルだけを購読する() async throws {
    // Given
    let layout = PaneLayout.split(
      id: UUID().uuidString, axis: .horizontal, ratio: 0.5,
      first: fileLayout(["left-hidden.rs", "left.rs"]),
      second: fileLayout(["right-hidden.rs", "right.rs"]))
    let (http, _, host) = try await filesPanel(layout)
    // When
    try await press(host, "ファイル")
    // Then
    try await expectStarts(http, paths: ["left.rs", "right.rs"])
  }

  func test_ファイルタブ_B002_同じpaneで選択を移すと購読対象が切り替わる() async throws {
    // Given
    let (http, _, host) = try await filesPanel(fileLayout(["other.rs", path]))
    try await press(host, "ファイル")
    try await expectStarts(http, paths: [path])
    let started = subscriptions(http)
    // When
    try await press(host, "other.rs")
    // Then
    try await expectStarts(http, paths: ["other.rs"], after: started.count)
    try await eventually {
      let stopped = http.requests("StopStateSubscription").compactMap {
        try? Releash_Client_V1_StopStateSubscriptionRequest(serializedBytes: $0.message ?? Data())
          .subscriptionID
      }
      return Set(started.map(\.subscriptionID)).isSubset(of: Set(stopped))
    }
  }

  private func snapshot(_ modified: String, mode: String) -> Releash_Client_V1_StatePayload {
    var payload = Releash_Client_V1_StatePayload()
    payload.fileTab.initialMode = mode
    payload.fileTab.view.textDiff.path = path
    payload.fileTab.view.textDiff.original = "one\nold\nthree\nfour\n"
    payload.fileTab.view.textDiff.modified = modified
    return payload
  }

  private func viewer(_ host: NSWindow) async throws -> WKWebView {
    let content = try XCTUnwrap(host.contentView)
    func current() -> WKWebView? {
      views(content, WKWebView.self).first {
        !$0.isHiddenOrHasHiddenAncestor && !$0.visibleRect.isEmpty
      }
    }
    try await eventually { current() != nil }
    let web = try XCTUnwrap(current())
    web.configuration.preferences.inactiveSchedulingPolicy = .none
    return web
  }

  func test_ファイルタブ_B004_選択したファイルの最新の内容と差分とThreadを表示する() async throws {
    // Given
    let (http, _, host) = try await filesPanel(fileLayout([path, "other.rs"], mode: "diff"))
    let args = ["/repo", path, "head"]
    try http.publish(
      snapshot("one\nold\nthree\nfour\n", mode: "diff"), target: "file-tab", args: args)
    try await press(host, "ファイル")
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
      "all('[data-line]').some(e => visible(e) && e.textContent.includes('updated-line')) && all('[data-line]').some(e => visible(e) && e.textContent.includes('old')) && text().includes('先の本文')"
    )
  }

  func test_ファイルタブ_B005_選び直したファイルでスクロール位置とThreadの下書きを保つ() async throws {
    // Given
    let (http, _, host) = try await filesPanel(fileLayout(["other.rs", path]))
    try await press(host, "ファイル")
    try await eventually {
      self.subscriptions(http).contains { $0.target == "file-tab" && $0.args.contains(self.path) }
    }
    let payload = snapshot((1...180).map { "line-\($0)" }.joined(separator: "\n"), mode: "file")
    let args = ["/repo", path, "head"]
    try http.publish(payload, target: "file-tab", args: args)
    let web = try await viewer(host)
    try await wait(web, "all('[data-line]').length > 0")
    try await selectLines(web)
    try await eventually { self.findElement(host, name: "Thread作成") != nil }
    try pressCreate(host)
    try await wait(web, "input()")
    _ = try await js(
      web,
      "input().value='未送信'; input().dispatchEvent(new Event('input',{bubbles:true})); document.querySelector('#viewport').scrollTop=85;"
    )
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
      "input()?.value === '未送信' && document.querySelector('#viewport').scrollTop === \(try XCTUnwrap(offset))"
    )
  }
}

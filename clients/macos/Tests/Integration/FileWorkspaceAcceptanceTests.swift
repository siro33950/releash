import AppKit
import ApplicationServices
import CryptoKit
import SwiftUI
import XCTest

@testable import Releash

final class FileWorkspaceAcceptanceTests: XCTestCase {
  @MainActor private func panel() async throws -> (TestHTTPClient, AppModel, ViewHarness) {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    let host = await ViewHarness(
      RightPanelView(expanded: .constant(true), model: model),
      size: NSSize(width: 1000, height: 700))
    try await host.press("ファイル")
    try await eventually {
      http.state.withLock { $0.subscriptions.contains { $0.target == "directory" } }
    }
    return (http, model, host)
  }

  private func entry(_ path: String, directory: Bool = false) -> Releash_Client_V1_DirectoryEntry {
    .with {
      $0.path = path
      $0.name = URL(fileURLWithPath: path).lastPathComponent
      $0.directory = directory
    }
  }

  private func publish(
    _ http: TestHTTPClient, directory: String = ".",
    entries: [Releash_Client_V1_DirectoryEntry]
  ) throws {
    var payload = Releash_Client_V1_StatePayload()
    payload.directory.entries = entries
    payload.directory.order = entries.map(\.path)
    try http.publish(payload, target: "directory", args: ["/repo", directory])
  }

  @MainActor private func enterSearch(_ value: String, in host: ViewHarness) async throws {
    try await host.focusTextField()
    let editor = try XCTUnwrap(host.window.firstResponder as? NSTextView)
    editor.insertText(
      value, replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
    await host.refresh()
  }

  @MainActor private func publishFile(_ name: String, http: TestHTTPClient) async throws {
    let args = ["/repo", name, "head"]
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "file-tab" && $0.args == args }
      }
    }
    var payload = Releash_Client_V1_StatePayload()
    payload.fileTab.initialMode = "file"
    payload.fileTab.view.textDiff.path = name
    payload.fileTab.view.textDiff.original = ""
    payload.fileTab.view.textDiff.modified = "fn main() {}\n"
    try http.publish(payload, target: "file-tab", args: args)
  }

  @MainActor private func views<T: NSView>(_ view: NSView, of type: T.Type) -> [T] {
    (view as? T).map { [$0] } ?? view.subviews.flatMap { views($0, of: type) }
  }

  @MainActor private func element(_ name: String, in host: ViewHarness, role: String? = nil) throws
    -> AXUIElement
  {
    func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
      var value: CFTypeRef?
      AXUIElementCopyAttributeValue(element, name as CFString, &value)
      return value
    }
    func find(_ element: AXUIElement) -> AXUIElement? {
      if role == nil || attribute(element, kAXRoleAttribute) as? String == role,
        [kAXIdentifierAttribute, kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute]
          .contains(where: {
            attribute(element, $0) as? String == name
          })
      {
        return element
      }
      for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] {
        if let found = find(child) { return found }
      }
      return nil
    }
    let app = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
    let windows = attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []
    guard
      let window = windows.first(where: {
        attribute($0, kAXTitleAttribute) as? String == host.window.title
      }),
      let found = find(window)
    else {
      throw NSError(
        domain: "FileWorkspaceAcceptance", code: 1,
        userInfo: [NSLocalizedDescriptionKey: "Missing accessibility element: \(name)"])
    }
    return found
  }

  @MainActor private func buttonTitles(in root: AXUIElement) throws -> [String] {
    func attribute(_ name: String) throws -> CFTypeRef? {
      var value: CFTypeRef?
      let result = AXUIElementCopyAttributeValue(root, name as CFString, &value)
      guard result == .success || result == .noValue || result == .attributeUnsupported else {
        throw NSError(domain: "Accessibility", code: Int(result.rawValue))
      }
      return value
    }
    let role = try XCTUnwrap(attribute(kAXRoleAttribute) as? String)
    var titles: [String] = []
    if role == kAXButtonRole {
      let title = try attribute(kAXTitleAttribute) as? String
      let description = try attribute(kAXDescriptionAttribute) as? String
      titles = [title, description].compactMap { $0 }
    }
    if let children = try attribute(kAXChildrenAttribute) {
      for child in try XCTUnwrap(children as? [AXUIElement]) {
        titles += try buttonTitles(in: child)
      }
    }
    return titles
  }

  @MainActor private func frame(_ name: String, in host: ViewHarness, role: String? = nil) throws
    -> CGRect
  {
    func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
      var value: CFTypeRef?
      AXUIElementCopyAttributeValue(element, name as CFString, &value)
      return value
    }
    let element = try element(name, in: host, role: role)
    var position = CGPoint.zero
    var size = CGSize.zero
    XCTAssertTrue(
      AXValueGetValue(
        try XCTUnwrap(attribute(element, kAXPositionAttribute)) as! AXValue, .cgPoint, &position))
    XCTAssertTrue(
      AXValueGetValue(
        try XCTUnwrap(attribute(element, kAXSizeAttribute)) as! AXValue, .cgSize, &size))
    return CGRect(origin: position, size: size)
  }

  @MainActor private func renderedControl(_ name: String, in host: ViewHarness) throws -> [UInt8] {
    let bounds = try frame(name, in: host)
    let screenHeight = try XCTUnwrap(NSScreen.screens.first).frame.maxY
    let screenRect = NSRect(
      x: bounds.minX, y: screenHeight - bounds.maxY, width: bounds.width, height: bounds.height)
    let view = try XCTUnwrap(host.window.contentView)
    view.layoutSubtreeIfNeeded()
    view.displayIfNeeded()
    let rect = view.convert(host.window.convertFromScreen(screenRect), from: nil)
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: rect))
    view.cacheDisplay(in: rect, to: bitmap)
    var pixels: [UInt8] = []
    for y in 0..<bitmap.pixelsHigh {
      for x in 0..<bitmap.pixelsWide {
        let color = try XCTUnwrap(bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB))
        pixels += [
          color.redComponent, color.greenComponent, color.blueComponent, color.alphaComponent,
        ]
        .map { UInt8((min(1, max(0, $0)) * 255).rounded()) }
      }
    }
    XCTAssertGreaterThan(
      Set(
        stride(from: 0, to: pixels.count, by: 4).map {
          Array(pixels[$0..<($0 + 4)])
        }
      ).count, 1, "コントロールの描画が単色で、アイコンを確認できません: \(name)")
    return pixels
  }

  @MainActor private func clickRow(_ path: String, in host: ViewHarness) async throws {
    try await eventuallyView(host) { host.element("file:\(path)") != nil }
    let row = try host.tableRow("file:\(path)")
    var position: CFTypeRef?
    var size: CFTypeRef?
    XCTAssertEqual(
      AXUIElementCopyAttributeValue(row, kAXPositionAttribute as CFString, &position), .success)
    XCTAssertEqual(
      AXUIElementCopyAttributeValue(row, kAXSizeAttribute as CFString, &size), .success)
    var origin = CGPoint.zero
    var dimensions = CGSize.zero
    XCTAssertTrue(AXValueGetValue(try XCTUnwrap(position) as! AXValue, .cgPoint, &origin))
    XCTAssertTrue(AXValueGetValue(try XCTUnwrap(size) as! AXValue, .cgSize, &dimensions))
    let screenHeight = try XCTUnwrap(NSScreen.screens.first).frame.maxY
    let point = host.window.convertPoint(
      fromScreen: NSPoint(
        x: origin.x + dimensions.width / 2, y: screenHeight - origin.y - dimensions.height / 2))
    let up = try XCTUnwrap(
      NSEvent.mouseEvent(
        with: .leftMouseUp, location: point, modifierFlags: [], timestamp: 0,
        windowNumber: host.window.windowNumber, context: nil, eventNumber: 1,
        clickCount: 1, pressure: 0))
    let down = try XCTUnwrap(
      NSEvent.mouseEvent(
        with: .leftMouseDown, location: point, modifierFlags: [], timestamp: 0,
        windowNumber: host.window.windowNumber, context: nil, eventNumber: 0,
        clickCount: 1, pressure: 1))
    NSApp.postEvent(up, atStart: true)
    host.window.sendEvent(down)
    await host.refresh()
  }

  @MainActor private func tabFrame(_ name: String, in host: ViewHarness) throws -> CGRect {
    try frame(name, in: host, role: kAXButtonRole)
  }

  @MainActor private func selected(_ name: String, in host: ViewHarness) throws -> Bool {
    var value: CFTypeRef?
    AXUIElementCopyAttributeValue(
      try element(name, in: host, role: kAXButtonRole), kAXSelectedAttribute as CFString, &value)
    return value as? Bool == true
  }

  @MainActor private func strip(_ name: String, in host: ViewHarness) throws -> [String] {
    var node = try element(name, in: host, role: kAXButtonRole)
    func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
      var value: CFTypeRef?
      AXUIElementCopyAttributeValue(element, name as CFString, &value)
      return value
    }
    while attribute(node, kAXRoleAttribute) as? String != kAXScrollAreaRole {
      node = try XCTUnwrap(attribute(node, kAXParentAttribute)) as! AXUIElement
    }
    func titles(_ element: AXUIElement) -> [String] {
      let title =
        attribute(element, kAXTitleAttribute) as? String ?? attribute(
          element, kAXDescriptionAttribute) as? String ?? ""
      let own =
        attribute(element, kAXRoleAttribute) as? String == kAXButtonRole
          && (title.hasSuffix(".rs") || title.hasPrefix("Terminal "))
        ? [title] : []
      return own
        + (attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []).flatMap(titles)
    }
    return titles(node)
  }

  @MainActor private func drag(from source: CGPoint, to destination: CGPoint) async {
    let pid = ProcessInfo.processInfo.processIdentifier
    let sent = await Task.detached {
      let events: [(CGEventType, CGPoint)] =
        [(.mouseMoved, source), (.leftMouseDown, source)]
        + (1...12).map { step in
          (
            .leftMouseDragged,
            CGPoint(
              x: source.x + (destination.x - source.x) * Double(step) / 12,
              y: source.y + (destination.y - source.y) * Double(step) / 12)
          )
        } + [(.leftMouseUp, destination)]
      for (type, point) in events {
        guard
          let event = CGEvent(
            mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: .left)
        else { return false }
        event.postToPid(pid)
        try? await Task.sleep(for: .milliseconds(30))
      }
      return true
    }.value
    XCTAssertTrue(sent)
  }

  @MainActor private func files(
    _ names: [String] = ["first.rs", "second.rs"], center: Bool = false
  )
    async throws -> (TestHTTPClient, AppModel, ViewHarness)
  {
    let (http, model, panelHost) = try await panel()
    let host: ViewHarness
    if center {
      let previous = http.state.withLock { Set($0.subscriptions.map(\.subscriptionID)) }
      panelHost.close()
      host = await ViewHarness(
        HStack {
          WorkspacePaneView(model: model)
          RightPanelView(expanded: .constant(true), model: model)
        }, size: NSSize(width: 1400, height: 700))
      try await host.press("ファイル")
      try await eventually {
        http.state.withLock {
          $0.subscriptions.contains {
            $0.target == "directory" && $0.args == ["/repo", "."]
              && !previous.contains($0.subscriptionID)
          }
        }
      }
    } else {
      host = panelHost
    }
    try publish(http, entries: names.map { entry($0) })
    for name in names {
      try await clickRow(name, in: host)
      try await eventuallyView(host) { (try? self.tabFrame(name, in: host)) != nil }
      try await publishFile(name, http: http)
      try await eventuallyView(host) { host.element("file-content") != nil }
    }
    return (http, model, host)
  }

  @MainActor private func splitFiles(in host: ViewHarness) async throws {
    try await host.press("first.rs")
    try await eventuallyView(host) { host.element("file-content") != nil }
    let source = try tabFrame("first.rs", in: host)
    let area = try frame("file-content", in: host)
    await drag(
      from: CGPoint(x: source.midX, y: source.midY), to: CGPoint(x: area.midX, y: area.maxY - 3))
    try await eventuallyView(host) { (try? self.strip("first.rs", in: host)) == ["first.rs"] }
  }

  @MainActor func test_ファイル面_b001_選んだ時点で一覧とタブ領域が左右に出る() async throws {
    // Given / When
    let (_, model, host) = try await panel()
    defer { host.close() }
    // Then
    let content = try XCTUnwrap(host.window.contentView)
    let outline = try XCTUnwrap(views(content, of: NSOutlineView.self).first)
    let split = try XCTUnwrap(
      views(content, of: NSSplitView.self)
        .first { $0.isVertical && $0.arrangedSubviews.count == 2 })
    XCTAssertTrue(outline.isDescendant(of: split.arrangedSubviews[0]))
    XCTAssertGreaterThan(split.arrangedSubviews[1].frame.width, 0)
    await model.stop()
  }

  @MainActor func test_ファイルタブ_b002_一覧から本文を開いても中央のタブは増えない() async throws {
    // Given
    let (http, model, host) = try await files([], center: true)
    defer { host.close() }
    let pane = try XCTUnwrap(model.panes.controller.focusedPaneId)
    model.panes.open(.terminal, in: pane)
    try await eventuallyView(host) { host.element("Terminal 1") != nil }
    let centerTabs = try strip("Terminal 1", in: host)
    try publish(http, entries: [entry("first.rs")])
    // When
    try await clickRow("first.rs", in: host)
    try await publishFile("first.rs", http: http)
    // Then
    try await eventuallyView(host) { host.element("file-content") != nil }
    XCTAssertEqual(try strip("first.rs", in: host), ["first.rs"])
    XCTAssertEqual(try strip("Terminal 1", in: host), centerTabs)
    XCTAssertEqual(host.element("file-path")?.label, "first.rs")
    XCTAssertGreaterThan(
      try frame("file-content", in: host).minX, try tabFrame("Terminal 1", in: host).maxX)
    await model.stop()
  }

  @MainActor func test_ファイルタブ_b003_既存のタブを増やさず選ぶ() async throws {
    // Given
    let (_, model, host) = try await files()
    defer { host.close() }
    XCTAssertTrue(try selected("second.rs", in: host))
    let tabs = try strip("first.rs", in: host)
    // When
    try await clickRow("first.rs", in: host)
    // Then
    try await eventuallyView(host) { (try? self.selected("first.rs", in: host)) == true }
    XCTAssertEqual(try strip("first.rs", in: host), tabs)
    await model.stop()
  }

  @MainActor func test_ファイルタブ_b004_端へのドラッグで領域を分割する() async throws {
    // Given
    let (_, model, host) = try await files()
    defer { host.close() }
    // When
    try await splitFiles(in: host)
    // Then
    XCTAssertEqual(try strip("first.rs", in: host), ["first.rs"])
    XCTAssertEqual(try strip("second.rs", in: host), ["second.rs"])
    XCTAssertGreaterThan(
      try tabFrame("first.rs", in: host).minY, try tabFrame("second.rs", in: host).maxY)
    await model.stop()
  }

  @MainActor func test_ファイルタブ_b005_別の領域のタブ列へ移す() async throws {
    // Given
    let (http, model, host) = try await files()
    defer { host.close() }
    try await splitFiles(in: host)
    try await host.press("first.rs")
    try publish(http, entries: [entry("first.rs"), entry("second.rs"), entry("third.rs")])
    try await clickRow("third.rs", in: host)
    try await eventuallyView(host) { (try? self.strip("first.rs", in: host).count) == 2 }
    // When
    let source = try tabFrame("third.rs", in: host)
    let target = try tabFrame("second.rs", in: host)
    await drag(
      from: CGPoint(x: source.midX, y: source.midY), to: CGPoint(x: target.midX, y: target.midY))
    // Then
    try await eventuallyView(host) {
      Set((try? self.strip("second.rs", in: host)) ?? []) == ["second.rs", "third.rs"]
    }
    XCTAssertEqual(try strip("first.rs", in: host), ["first.rs"])
    await model.stop()
  }

  @MainActor func test_ファイルタブ_b006_フォーカスのある領域へ開く() async throws {
    // Given
    let (http, model, host) = try await files()
    defer { host.close() }
    try await splitFiles(in: host)
    try await host.press("first.rs")
    let other = try strip("second.rs", in: host)
    try publish(http, entries: [entry("first.rs"), entry("second.rs"), entry("third.rs")])
    // When
    try await clickRow("third.rs", in: host)
    // Then
    try await eventuallyView(host) {
      Set((try? self.strip("first.rs", in: host)) ?? []) == ["first.rs", "third.rs"]
    }
    XCTAssertEqual(try strip("second.rs", in: host), other)
    await model.stop()
  }

  @MainActor func test_ファイルタブ_b007_閉じたタブだけが消える() async throws {
    // Given
    let (_, model, host) = try await files()
    defer { host.close() }
    let tab = try element("first.rs", in: host, role: kAXButtonRole)
    var actions: CFArray?
    XCTAssertEqual(AXUIElementCopyActionNames(tab, &actions), .success)
    let close = try XCTUnwrap(
      (actions as? [String])?.first { action in
        var description: CFString?
        AXUIElementCopyActionDescription(tab, action as CFString, &description)
        return action == "Close Tab" || description as String? == "Close Tab"
      })
    // When
    XCTAssertEqual(AXUIElementPerformAction(tab, close as CFString), .success)
    // Then
    try await eventuallyView(host) { (try? self.strip("second.rs", in: host)) == ["second.rs"] }
    await model.stop()
  }

  @MainActor func test_ファイルタブ_b008_同じ列の後ろへ並べ替える() async throws {
    // Given
    let (_, model, host) = try await files()
    defer { host.close() }
    XCTAssertEqual(try strip("first.rs", in: host), ["first.rs", "second.rs"])
    let source = try tabFrame("first.rs", in: host)
    let target = try tabFrame("second.rs", in: host)
    // When
    await drag(
      from: CGPoint(x: source.midX, y: source.midY),
      to: CGPoint(x: target.maxX + 15, y: target.midY)
    )
    // Then
    try await eventuallyView(host) {
      (try? self.strip("first.rs", in: host)) == ["second.rs", "first.rs"]
    }
    await model.stop()
  }

  @MainActor func test_ファイルタブ_b009_b010_中央との間では移動しない() async throws {
    // Given
    let (_, model, host) = try await files(["first.rs"], center: true)
    defer { host.close() }
    let pane = try XCTUnwrap(model.panes.controller.focusedPaneId)
    model.panes.open(.terminal, in: pane)
    try await eventuallyView(host) { host.element("Terminal 1") != nil }
    let file = try tabFrame("first.rs", in: host)
    let terminal = try frame("Terminal 1", in: host, role: kAXButtonRole)
    // When / Then
    for (source, target) in [(file, terminal), (terminal, file)] {
      await drag(
        from: CGPoint(x: source.midX, y: source.midY), to: CGPoint(x: target.midX, y: target.midY))
      // Observe through subsequent event-loop turns so a delayed drop cannot pass unnoticed.
      let deadline = ContinuousClock.now.advanced(by: .seconds(1))
      repeat {
        try await Task.sleep(for: .milliseconds(20))
        await host.refresh()
        XCTAssertEqual(try strip("first.rs", in: host), ["first.rs"])
        XCTAssertEqual(try strip("Terminal 1", in: host), ["Terminal 1"])
        XCTAssertGreaterThan(
          try tabFrame("first.rs", in: host).minX,
          try tabFrame("Terminal 1", in: host).maxX)
      } while ContinuousClock.now < deadline
    }
    await model.stop()
  }

  @MainActor func test_ファイル面_b014_worktreeを選び直すと開いていたタブが戻る() async throws {
    // Given
    let (http, model, host) = try await files(["first.rs"])
    defer { host.close() }
    // When
    model.selectWorktree("/other")
    try await eventuallyView(host) {
      model.selectedWorktree == "/other" && model.layoutLoaded
        && http.state.withLock {
          $0.subscriptions.contains { $0.target == "directory" && $0.args.first == "/other" }
        }
    }
    try await eventuallyView(host) {
      (try? self.element("first.rs", in: host, role: kAXButtonRole)) == nil
    }
    model.selectWorktree("/repo")
    try await eventually { model.selectedWorktree == "/repo" && model.layoutLoaded }
    try await host.press("ファイル")
    // Then
    try await eventuallyView(host) { (try? self.strip("first.rs", in: host)) == ["first.rs"] }
    await model.stop()
  }

  @MainActor func test_ファイル面_b015_b016_別の面へ切り替えて戻る() async throws {
    // Given
    let (http, model, host) = try await files(["first.rs"])
    defer { host.close() }
    for surface in ["Node", "Source Control", "Thread"] {
      // When / Then
      try await host.press(surface)
      try await eventuallyView(host) {
        host.element("file:first.rs") == nil && host.element("file-path") == nil
          && (try? self.element("first.rs", in: host, role: kAXButtonRole)) == nil
      }
      let previous = http.state.withLock { Set($0.subscriptions.map(\.subscriptionID)) }
      try await host.press("ファイル")
      try await eventually {
        http.state.withLock {
          $0.subscriptions.contains {
            $0.target == "directory" && $0.args == ["/repo", "."]
              && !previous.contains($0.subscriptionID)
          }
        }
      }
      try publish(http, entries: [entry("first.rs")])
      try await eventuallyView(host) {
        host.element("file:first.rs") != nil && host.element("file-path") != nil
          && (try? self.strip("first.rs", in: host)) == ["first.rs"]
      }
    }
    await model.stop()
  }

  @MainActor func test_ファイル面_b031_境界を動かすと一覧とタブ領域の幅が変わる() async throws {
    // Given
    let (_, model, host) = try await panel()
    defer { host.close() }
    let split = try XCTUnwrap(
      views(try XCTUnwrap(host.window.contentView), of: NSSplitView.self)
        .first { $0.isVertical && $0.arrangedSubviews.count == 2 })
    for offset in [60.0, -60.0] {
      let left = split.arrangedSubviews[0].frame.width
      let right = split.arrangedSubviews[1].frame.width
      let local = NSPoint(
        x: split.arrangedSubviews[0].frame.maxX + split.dividerThickness / 2, y: split.bounds.midY)
      let screen = host.window.convertPoint(toScreen: split.convert(local, to: nil))
      let source = CGPoint(
        x: screen.x, y: try XCTUnwrap(NSScreen.screens.first).frame.maxY - screen.y)
      // When
      await drag(from: source, to: CGPoint(x: source.x + offset, y: source.y))
      // Then
      try await eventuallyView(host) {
        (split.arrangedSubviews[0].frame.width - left) * offset > 0
          && (split.arrangedSubviews[1].frame.width - right) * offset < 0
      }
    }
    await model.stop()
  }

  @MainActor func test_ファイル面_b017_b027_b035_ヘッダーと外部エディタへの対象を確認する() async throws {
    // Given
    let (http, model, host) = try await panel()
    defer { host.close() }
    try publish(http, entries: [entry("file.rs")])
    // When
    try await clickRow("file.rs", in: host)
    // Then
    try await eventuallyView(host) {
      host.element("差分のみ") != nil && host.element("IDE で開く") != nil
        && host.element("file-mode") != nil && host.element("file-open-editor") != nil
    }
    let search = try XCTUnwrap(
      views(try XCTUnwrap(host.window.contentView), of: NSTextField.self).first { $0.isEditable })
    let searchFrame = host.window.convertToScreen(search.convert(search.bounds, to: nil))
    let changedFrame = try frame("差分のみ", in: host)
    let editorFrame = try frame("IDE で開く", in: host)
    XCTAssertLessThanOrEqual(searchFrame.maxX, changedFrame.minX)
    XCTAssertLessThanOrEqual(changedFrame.maxX, editorFrame.minX)
    for (actual, reference) in [("差分のみ", "file-mode"), ("IDE で開く", "file-open-editor")] {
      XCTAssertEqual(host.element(actual)?.role, host.element(reference)?.role)
      let actualFrame = try frame(actual, in: host)
      let referenceFrame = try frame(reference, in: host)
      XCTAssertEqual(actualFrame.width, referenceFrame.width, accuracy: 0.5)
      XCTAssertEqual(actualFrame.height, referenceFrame.height, accuracy: 0.5)
      let actualPixels = try renderedControl(actual, in: host)
      let referencePixels = try renderedControl(reference, in: host)
      XCTAssertTrue(
        actualPixels == referencePixels,
        "\(actual) と \(reference) のアイコン・表示ラベル・スタイルの描画が一致しません")
    }
    // When
    try await host.press("IDE で開く")
    // Then
    try await eventually { http.requests("OpenInEditor").count == 1 }
    let worktree = try Releash_Client_V1_OpenInEditorRequest(
      serializedBytes: XCTUnwrap(http.requests("OpenInEditor").first?.message))
    XCTAssertEqual(worktree.worktreePath, "/repo")
    XCTAssertFalse(worktree.hasFile)
    // When
    try await host.press("file-open-editor")
    // Then
    try await eventually { http.requests("OpenInEditor").count == 2 }
    let file = try Releash_Client_V1_OpenInEditorRequest(
      serializedBytes: XCTUnwrap(http.requests("OpenInEditor").last?.message))
    XCTAssertEqual(file.worktreePath, "/repo")
    XCTAssertEqual(file.file, "file.rs")
    await model.stop()
  }

  @MainActor func test_ファイル一覧_b018_b022_b023_b025_b026_絞り込みをやめると元の開閉状態へ戻る() async throws {
    for search in [true, false] {
      // Given
      let (http, model, host) = try await panel()
      defer { host.close() }
      try publish(
        http, entries: [entry("opened", directory: true), entry("closed", directory: true)])
      try await eventuallyView(host) { host.element("file:opened") != nil }
      try await host.setRowExpanded("file:opened", true)
      try await eventually {
        http.state.withLock {
          $0.subscriptions.contains { $0.target == "directory" && $0.args.last == "opened" }
        }
      }
      try publish(http, directory: "opened", entries: [entry("opened/ordinary.rs")])
      try await eventuallyView(host) { host.element("file:opened/ordinary.rs") != nil }
      if search {
        try await enterSearch("needle", in: host)
      } else {
        try await host.press("差分のみ")
      }
      let args = search ? ["/repo", "name=needle"] : ["/repo", "changed=true"]
      try await eventually {
        http.state.withLock {
          $0.subscriptions.contains { $0.target == "filtered-files" && $0.args == args }
        }
      }
      var payload = Releash_Client_V1_StatePayload()
      payload.filteredFiles.directories = [
        .with {
          $0.path = "."
          $0.entries = [entry("closed", directory: true)]
        },
        .with {
          $0.path = "closed"
          $0.entries = [entry("closed/needle.rs")]
        },
      ]
      try http.publish(payload, target: "filtered-files", args: args)
      try await eventuallyView(host) { host.element("file:closed/needle.rs") != nil }
      var expanded: CFTypeRef?
      XCTAssertEqual(
        AXUIElementCopyAttributeValue(
          try host.tableRow("file:closed"), kAXDisclosingAttribute as CFString, &expanded), .success
      )
      XCTAssertEqual(expanded as? Bool, true)
      if search {
        try await host.press("差分のみ")
        let combined = ["/repo", "name=needle", "changed=true"]
        try await eventually {
          http.state.withLock {
            $0.subscriptions.contains { $0.target == "filtered-files" && $0.args == combined }
          }
        }
        try http.publish(payload, target: "filtered-files", args: combined)
        try await eventuallyView(host) { host.element("file:closed/needle.rs") != nil }
        try await host.press("差分のみ")
        try await eventually {
          http.state.withLock {
            $0.subscriptions.contains { $0.target == "filtered-files" && $0.args == args }
          }
        }
        try http.publish(payload, target: "filtered-files", args: args)
      }
      // When
      if search {
        try await enterSearch("", in: host)
      } else {
        try await host.press("差分のみ")
      }
      try publish(
        http, entries: [entry("opened", directory: true), entry("closed", directory: true)])
      try publish(http, directory: "opened", entries: [entry("opened/ordinary.rs")])
      // Then
      try await eventuallyView(host) { host.element("file:opened/ordinary.rs") != nil }
      XCTAssertNil(host.element("file:closed/needle.rs"))
      for (name, expanded) in [("opened", true), ("closed", false)] {
        var value: CFTypeRef?
        XCTAssertEqual(
          AXUIElementCopyAttributeValue(
            try host.tableRow("file:\(name)"), kAXDisclosingAttribute as CFString, &value), .success
        )
        XCTAssertEqual(value as? Bool, expanded)
      }
      await model.stop()
    }
  }
  @MainActor func test_ファイル検索_b032_b033_一部の結果のときだけ絞り込みを促す() async throws {
    // Given
    let (http, model, host) = try await panel()
    defer { host.close() }
    try await enterSearch("needle", in: host)
    try await eventually {
      http.state.withLock { $0.subscriptions.contains { $0.target == "filtered-files" } }
    }
    for truncated in [false, true, false] {
      var payload = Releash_Client_V1_StatePayload()
      payload.filteredFiles.truncated = truncated
      payload.filteredFiles.directories = [
        .with {
          $0.path = "."
          $0.entries = (0..<512).map { entry("needle-\($0).rs") }
        }
      ]
      // When
      try http.publish(payload, target: "filtered-files")
      // Then
      try await eventuallyView(host) {
        guard host.element("file:needle-0.rs") != nil else { return false }
        let partial = host.texts().contains { $0.contains("一部") }
        let refine = host.texts().contains { $0.contains("入力") && $0.contains("絞") }
        return partial == truncated && refine == truncated
      }
      let outline = try XCTUnwrap(
        views(try XCTUnwrap(host.window.contentView), of: NSOutlineView.self).first)
      XCTAssertEqual(outline.numberOfRows, 512)
      if truncated {
        let message = try XCTUnwrap(host.texts().first { $0.contains("一部") })
        XCTAssertGreaterThanOrEqual(
          try frame(message, in: host).minY, try frame("差分のみ", in: host).maxY)
      }
    }
    await model.stop()
  }

  @MainActor func test_ファイル検索_b034_続けて入力したとき途中の検索結果を出さない() async throws {
    // Given
    let (http, model, host) = try await panel()
    defer { host.close() }
    try publish(http, entries: [entry("closed", directory: true)])
    try await eventuallyView(host) { host.element("file:closed") != nil }
    let candidates = [entry("closed/alpha.rs"), entry("closed/ab.rs")]
    let responder = Task { @MainActor in
      var delivered = Set<String>()
      while !Task.isCancelled {
        let subscriptions = http.state.withLock {
          $0.subscriptions.filter { $0.target == "filtered-files" }
        }
        for subscription in subscriptions
        where delivered.insert(subscription.subscriptionID).inserted {
          var payload = Releash_Client_V1_StatePayload()
          let matches =
            subscription.args.last == "name=a"
            ? candidates : (subscription.args.last == "name=ab" ? [candidates[1]] : [])
          payload.filteredFiles.directories = [
            .with {
              $0.path = "."
              $0.entries = [entry("closed", directory: true)]
            },
            .with {
              $0.path = "closed"
              $0.entries = matches
            },
          ]
          try http.publish(payload, target: "filtered-files", args: subscription.args)
        }
        try await Task.sleep(for: .milliseconds(10))
      }
    }
    defer { responder.cancel() }
    try await host.focusTextField()
    let editor = try XCTUnwrap(host.window.firstResponder as? NSTextView)
    var observedFirstOnly = false
    var observations = 0
    let observer = Task { @MainActor in
      while !Task.isCancelled {
        await host.refresh()
        observations += 1
        observedFirstOnly = observedFirstOnly || host.element("file:closed/alpha.rs") != nil
        try await Task.sleep(for: .milliseconds(1))
      }
    }
    defer { observer.cancel() }
    // When: allow rendering and the observer to run between keystrokes.
    let firstInput = ContinuousClock.now
    editor.insertText("a", replacementRange: NSRange(location: NSNotFound, length: 0))
    try await Task.sleep(for: .milliseconds(100))
    let interval = firstInput.duration(to: .now)
    XCTAssertLessThan(interval, .milliseconds(250), "入力間隔が受入条件を外れました")
    XCTAssertGreaterThan(observations, 0, "文字入力の間に画面が観測されませんでした")
    editor.insertText("b", replacementRange: NSRange(location: NSNotFound, length: 0))
    // Then
    try await eventuallyView(host) { host.element("file:closed/ab.rs") != nil }
    observedFirstOnly = observedFirstOnly || host.element("file:closed/alpha.rs") != nil
    XCTAssertFalse(observedFirstOnly, "途中の一覧に alpha.rs が表示されました")
    XCTAssertNil(host.element("file:closed/alpha.rs"))
    await model.stop()
  }

  @MainActor func test_ファイル復元_b028_旧版の中央ファイルタブをどちらの領域にも出さない() async throws {
    // Given: workspace.state_changed v1 from the schema before 435fda4f.
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let data = root.appendingPathComponent("data")
    let repo = root.appendingPathComponent("repo")
    for directory in [data, repo] {
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }
    let helpers = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers")
    var daemon: Process?
    defer {
      if let daemon, daemon.isRunning {
        daemon.terminate()
        daemon.waitUntilExit()
      }
      try? FileManager.default.removeItem(at: root)
    }
    func run(_ executable: String, _ arguments: [String]) throws -> Data {
      let process = Process()
      process.executableURL = URL(fileURLWithPath: executable)
      process.arguments = arguments
      process.currentDirectoryURL = repo
      let output = Pipe()
      process.standardOutput = output
      process.standardError = FileHandle.nullDevice
      try process.run()
      let bytes = output.fileHandleForReading.readDataToEndOfFile()
      process.waitUntilExit()
      guard process.terminationStatus == 0 else {
        throw NSError(domain: "LegacyFixture", code: Int(process.terminationStatus))
      }
      return bytes
    }
    _ = try run("/usr/bin/git", ["init"])
    _ = try run(
      "/usr/bin/git",
      [
        "-c", "user.name=Acceptance", "-c", "user.email=test@example.invalid", "-c",
        "commit.gpgsign=false", "commit", "--allow-empty", "-m", "initial",
      ])
    try "legacy\n".write(
      to: repo.appendingPathComponent("legacy.rs"), atomically: true, encoding: .utf8)
    let discovery = data.appendingPathComponent("client-api.json")
    func start() async throws {
      let process = Process()
      process.executableURL = helpers.appendingPathComponent("releashd")
      process.arguments = ["--data-dir", data.path]
      process.currentDirectoryURL = data
      var environment = ProcessInfo.processInfo.environment
      environment["RELEASH_DATA_DIR"] = data.path
      environment["HOME"] = root.path
      environment["CODEX_HOME"] = root.appendingPathComponent("codex").path
      environment["CLAUDE_CONFIG_DIR"] = root.appendingPathComponent("claude").path
      environment["XDG_CONFIG_HOME"] = root.appendingPathComponent("config").path
      environment["SHELL"] = "/bin/sh"
      process.environment = environment
      process.standardInput = FileHandle.nullDevice
      process.standardOutput = FileHandle.nullDevice
      process.standardError = FileHandle.nullDevice
      try process.run()
      daemon = process
      try await eventually {
        guard let bytes = try? Data(contentsOf: discovery),
          let value = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any]
        else { return false }
        return (value["pid"] as? NSNumber)?.int32Value == process.processIdentifier
      }
    }
    let status = try JSONSerialization.data(withJSONObject: [
      "running": true, "compatibility": "compatible", "release_compatibility": "compatible",
      "discovery_file": discovery.path,
    ])
    let cli = CLI(executable: helpers.appendingPathComponent("releash"), run: { _, _ in status })
    try await start()
    let writer = AppModel(cli: cli)
    await writer.start()
    do {
      let client = try XCTUnwrap(writer.client)
      var save = Releash_Client_V1_SaveWorkspaceStateRequest()
      save.worktreeName = repo.path
      save.state.version = 1
      save.state.etag = "0"
      _ = try await client.saveWorkspaceState(request: save, headers: writer.headers).result.get()
    } catch {
      await writer.stop()
      throw error
    }
    await writer.stop()
    daemon?.terminate()
    try await eventually { daemon?.isRunning == false }
    daemon?.waitUntilExit()
    let legacy = try XCTUnwrap(
      Data(
        base64Encoded:
          "eLd7InZlcnNpb24iOjEsInBhbmVzIjp7ImtpbmQiOiJwYW5lIiwiaWQiOiJwYW5lIiwiYWN0aXZlX3RhYiI6Im9sZC1maWxlIiwidGFicyI6W3siaWQiOiJvbGQtZmlsZSIsImtpbmQiOiJmaWxlIiwiZmlsZV9wYXRoIjoibGVnYWN5LnJzIiwiZmlsZV9tb2RlIjoiZGlmZiIsImZpbGVfYmFzZSI6ImJyYW5jaC1iYXNlIn1dfX0="
      ))
    let payload = legacy.map { String(format: "%02x", $0) }.joined()
    let hash = SHA256.hash(data: legacy).map { String(format: "%02x", $0) }.joined()
    let changed = try run(
      "/usr/bin/sqlite3",
      [
        data.appendingPathComponent("local-event-store.sqlite3").path,
        "UPDATE events SET payload = X'\(payload)', payload_sha256 = X'\(hash)' WHERE event_type = 'workspace.state_changed'; SELECT changes();",
      ])
    XCTAssertEqual(
      String(decoding: changed, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines), "1")
    try await start()
    let model = AppModel(cli: cli)
    await model.start()
    do {
      let host = await ViewHarness(
        HStack {
          WorkspacePaneView(model: model)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("acceptance-center")
          RightPanelView(expanded: .constant(true), model: model)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("acceptance-files")
        }, size: NSSize(width: 1400, height: 700))
      defer { host.close() }
      // When
      model.selectWorktree(repo.path)
      try await eventually { model.layoutLoaded }
      try await host.press("ファイル")
      try await eventuallyView(host) { host.element("file:legacy.rs") != nil }
      // Then: inspect both displayed regions, not the DTO or the database.
      let content = try XCTUnwrap(host.window.contentView)
      let split = try XCTUnwrap(
        views(content, of: NSSplitView.self)
          .first { $0.isVertical && $0.arrangedSubviews.count == 2 })
      XCTAssertGreaterThan(split.arrangedSubviews[1].frame.width, 0)
      XCTAssertGreaterThan(split.arrangedSubviews[1].frame.height, 0)
      for region in ["acceptance-center", "acceptance-files"] {
        let bounds = try frame(region, in: host)
        XCTAssertGreaterThan(bounds.width, 0)
        XCTAssertGreaterThan(bounds.height, 0)
        let titles = try buttonTitles(in: element(region, in: host))
        XCTAssertFalse(titles.contains("legacy.rs"), "\(region) に旧版のタブが表示されています")
      }
    } catch {
      await model.stop()
      throw error
    }
    await model.stop()
  }

}

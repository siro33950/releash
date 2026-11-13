import AppKit
import ApplicationServices
import SwiftProtobuf
import SwiftUI
import Vision
import XCTest

@testable import Releash

final class SidebarAcceptanceTests: XCTestCase {
  @MainActor private func repository(_ names: [String]) -> Releash_Client_V1_WorkspaceRepositoryList
  {
    .with {
      $0.path = "/acceptance"
      $0.branches.items = names.map { name in
        .with {
          $0.name = name
          $0.worktreePath = "/acceptance/" + name
        }
      }
      $0.worktrees.items = names.map { name in .with { $0.path = "/acceptance/" + name } }
    }
  }

  @MainActor func test_B001_B002_B018_上端の作成と下端の追加更新を右寄せする() async throws {
    // Given
    let model = AppModel()
    model.repositories = [repository(["main"])]
    // When
    let host = await ViewHarness(
      SidebarView(model: model, creating: .constant(false)),
      size: NSSize(width: 340, height: 700))
    defer { host.close() }
    try await eventuallyView(host) { host.element("Worktreeを作成") != nil }
    // Then
    XCTAssertNil(host.element("Workspaces"))
    let create = try frame("Worktreeを作成", in: host)
    let add = try frame("Repositoryを追加", in: host)
    let refresh = try frame("更新", in: host)
    let bounds = try XCTUnwrap(host.window.contentView).bounds
    XCTAssertLessThan(create.midY, 40)
    XCTAssertGreaterThan(create.midX, bounds.maxX - 50)
    XCTAssertGreaterThan(add.midY, bounds.maxY - 50)
    XCTAssertEqual(add.midY, refresh.midY, accuracy: 1)
    XCTAssertGreaterThan(refresh.midX, add.midX)
    XCTAssertGreaterThan(add.midX, bounds.midX)
    XCTAssertGreaterThan(refresh.midX, bounds.maxX - 50)
    let buttons = try axElements(in: host).filter {
      attribute($0, kAXRoleAttribute) as? String == kAXButtonRole
    }
    let buttonFrames = try buttons.map { try frame($0, in: host) }
    XCTAssertEqual(buttonFrames.filter { $0.midY >= 0 && $0.midY < 40 }.count, 1)
    XCTAssertEqual(buttonFrames.filter { $0.midY > bounds.maxY - 50 }.count, 2)
    for (label, icon) in [
      ("Worktreeを作成", "plus"), ("Repositoryを追加", "folder-plus"), ("更新", "refresh-cw"),
    ] {
      try await assertIcon(icon, in: host, element: label)
    }
    let connected = try await testModel(TestHTTPClient())
    connected.repositories = model.repositories
    let root = await ViewHarness(RootView(model: connected))
    defer { root.close() }
    try await eventuallyView(root) { root.element("Worktreeを作成") != nil }
    XCTAssertEqual(
      try frame("Worktreeを作成", in: root).midY,
      try frame("閉じる", in: root).midY, accuracy: 1)
    await connected.stop()
  }

  @MainActor func test_B003_更新操作後に購読した一覧が表示される() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.repositories = [repository(["before"])] }
    let model = try await testModel(http)
    let host = await ViewHarness(SidebarView(model: model, creating: .constant(false)))
    defer { host.close() }
    try await eventuallyView(host) { host.element("worktree:before") != nil }
    // When
    try await host.press("更新")
    try await eventually { http.requests("RefreshWorkspaces").count == 1 }
    var payload = Releash_Client_V1_StatePayload()
    payload.workspaces.repositories.items = [repository(["after"])]
    try http.publish(payload, target: "workspaces")
    // Then
    try await eventuallyView(host) {
      host.element("worktree:after") != nil && host.element("worktree:before") == nil
    }
    await model.stop()
  }

  @MainActor func test_B004_B005_B018_リポジトリの警告を見出しの右端に表示する() async throws {
    // Given
    let model = AppModel()
    var repo = repository(["main"])
    repo.status.error = "Repository fetch failed"
    model.repositories = [repo]
    // When
    let host = await ViewHarness(
      SidebarView(model: model, creating: .constant(false)),
      size: NSSize(width: 340, height: 700))
    defer { host.window.close() }
    try await eventuallyView(host) { host.element("repository:/acceptance") != nil }
    // Then
    let heading = try frame("repository:/acceptance", in: host)
    let region = NSRect(
      x: heading.maxX - 30, y: heading.minY - 5, width: 30, height: heading.height + 10)
    try await assertIcon("triangle-alert", in: host, rect: region)
    XCTAssertFalse(host.texts().contains("Repository fetch failed"))
    try await assertWarningTooltip(["Repository fetch failed"], in: host, rect: region)
  }

  @MainActor func test_B006_B007_B010_B018_PRと警告が先頭行に状態色で並ぶ() async throws {
    // Given
    let model = AppModel()
    var repo = repository(["open", "draft", "merged", "closed", "warning", "empty"])
    for index in 0..<5 {
      repo.branches.items[index].prState = index == 4 ? "open" : repo.branches.items[index].name
      repo.branches.items[index].prStateNumber = UInt64(index + 1)
      repo.branches.items[index].dirtyCount = 3
    }
    repo.branches.items[4].dirtyCountError = "Dirty count failed"
    model.repositories = [repo]
    // When
    let host = await ViewHarness(
      SidebarView(model: model, creating: .constant(false)),
      size: NSSize(width: 340, height: 1000))
    defer { host.close() }
    try await eventuallyView(host) { host.element("worktree:empty") != nil }
    // Then
    let empty = try frame("worktree:empty", in: host)
    for (name, icon, color) in [
      ("open", "git-pull-request", Color.green),
      ("draft", "git-pull-request-draft", Color.gray),
      ("merged", "git-merge", Color.purple),
      ("closed", "git-pull-request-closed", Color.red),
      ("warning", "git-pull-request", Color.green),
    ] {
      let card = try frame("worktree:" + name, in: host)
      let firstRight = NSRect(
        x: card.midX, y: card.minY, width: card.width / 2, height: card.height / 2)
      try await assertIcon(icon, in: host, rect: firstRight, color: color)
      XCTAssertEqual(card.height, empty.height, accuracy: 1)
      if name == "warning" { try await assertIcon("triangle-alert", in: host, rect: firstRight) }
      XCTAssertFalse(host.element("worktree:" + name)?.label.contains("3 changes") == true)
      XCTAssertFalse(try containsIcon("circle-check", in: host, rect: card))
    }
  }

  @MainActor func test_B008_B009_失敗を一つの警告にまとめ全エラーをヘルプに出す() async throws {
    // Given
    let model = AppModel()
    var repo = repository(["dirty", "pr", "worktree", "all", "empty"])
    for index in [0, 3] { repo.branches.items[index].dirtyCountError = "Dirty fetch failed" }
    for index in [1, 3] { repo.branches.items[index].pullRequestError = "PR fetch failed" }
    for index in [2, 3] { repo.worktrees.items[index].status.error = "Worktree fetch failed" }
    model.repositories = [repo]
    // When
    let host = await ViewHarness(
      SidebarView(model: model, creating: .constant(false)),
      size: NSSize(width: 340, height: 1000))
    defer { host.window.close() }
    try await eventuallyView(host) { host.element("worktree:empty") != nil }
    // Then
    let empty = try frame("worktree:empty", in: host)
    for name in ["dirty", "pr", "worktree", "all"] {
      let card = try frame("worktree:" + name, in: host)
      let firstRight = NSRect(
        x: card.midX, y: card.minY, width: card.width / 2, height: card.height / 2)
      try await assertIcon("triangle-alert", in: host, rect: firstRight)
      XCTAssertEqual(try iconCount("triangle-alert", in: host, rect: card), 1)
      XCTAssertEqual(card.height, empty.height, accuracy: 1)
      for text in ["Changes unavailable", "PR unavailable", "Worktree fetch failed"] {
        XCTAssertFalse(host.element("worktree:" + name)?.label.contains(text) == true)
      }
    }
    try await assertWarningTooltip(
      [
        "Changes unavailable", "Dirty fetch failed", "PR unavailable", "PR fetch failed",
        "Worktree fetch failed",
      ],
      in: host, rect: try frame("worktree:all", in: host))
  }

  @MainActor func test_B011_B012_B013_B014_実行行と空行と削除行を配置する() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    var repo = repository(["empty", "session", "workflow", "deleting"])
    let session = Releash_Client_V1_WorktreeExecutionSummary.with {
      $0.id = "session"
      $0.title = "Session title"
      $0.status = "active"
    }
    repo.worktrees.items[1].executions = [session]
    repo.worktrees.items[2].executions = [
      .with {
        $0.id = "workflow"
        $0.title = "Workflow title"
        $0.isWorkflow = true
        $0.status = "active"
        $0.nodeCount = 3
      }
    ]
    repo.worktrees.items[3].executions = [
      .with {
        $0.id = "deleting-session"
        $0.title = "Deleting session"
        $0.status = "active"
      }
    ]
    model.repositories = [repo]
    let host = await ViewHarness(
      SidebarView(model: model, creating: .constant(false)),
      size: NSSize(width: 340, height: 1000))
    defer { host.close() }
    try await eventuallyView(host) { host.element("worktree:deleting") != nil }
    // When
    try await host.menu("worktree:deleting", selecting: "削除") { _ in }
    try await eventually { host.window.attachedSheet?.isVisible == true }
    try await host.pressAlertButton("削除")
    try await eventually { http.requests("RemoveWorktree").count == 1 }
    let removal = try Releash_Client_V1_RemoveWorktreeRequest(
      serializedBytes: XCTUnwrap(http.requests("RemoveWorktree").last?.message))
    XCTAssertEqual(removal.worktreePath, "/acceptance/deleting")
    repo.branches.items[3].isDeleting = true
    var payload = Releash_Client_V1_StatePayload()
    payload.workspaces.repositories.items = [repo]
    try http.publish(payload, target: "workspaces")
    try await eventuallyView(host) {
      host.element("worktree:deleting")?.label.contains("削除中") == true
    }
    // Then
    let empty = try frame("worktree:empty", in: host)
    let sessionCard = try frame("worktree:session", in: host)
    let workflowCard = try frame("worktree:workflow", in: host)
    let deleting = try frame("worktree:deleting", in: host)
    XCTAssertEqual(empty.height, sessionCard.height, accuracy: 1)
    XCTAssertTrue(host.element("worktree:session")?.label.contains("Session title") == true)
    XCTAssertTrue(
      host.element("worktree:workflow")?.label.contains("Workflow title · 3 nodes") == true)
    for provider in ["Claude", "Codex"] {
      XCTAssertFalse(host.element("worktree:session")?.label.contains(provider) == true)
    }
    try await assertIcon("bot", in: host, element: "worktree:session")
    try await assertIcon("waypoints", in: host, element: "worktree:workflow")
    let sessionStyle = try textStyle("Session title", in: host, rect: sessionCard)
    let workflowStyle = try textStyle("Workflow title", in: host, rect: workflowCard)
    let workflowRaster = try raster(in: host, rect: workflowCard)
    for side in 6...12 {
      let square = NSHostingView(
        rootView: Rectangle().fill(.black)
          .frame(width: CGFloat(side), height: CGFloat(side)).padding(2).background(.white))
      let reference = try referenceRaster(square)
      XCTAssertNotNil(reference.find(reference, color: nil))
      XCTAssertNil(
        workflowRaster.find(reference, color: nil),
        "Workflow card must not display session status squares (\(side) pt)")
    }
    XCTAssertEqual(sessionStyle.size, workflowStyle.size)
    XCTAssertLessThan(
      ((sessionStyle.color - workflowStyle.color) * (sessionStyle.color - workflowStyle.color))
        .sum(), 0.01)
    let suffix = try textStyle("· 3 nodes", in: host, rect: workflowCard).color
    XCTAssertEqual(suffix.x, suffix.y, accuracy: 0.05)
    XCTAssertEqual(suffix.y, suffix.z, accuracy: 0.05)
    XCTAssertGreaterThan(suffix.sum() / 3, 0.2)
    XCTAssertLessThan(suffix.sum() / 3, 0.8)
    let deletingRow = NSRect(
      x: deleting.minX, y: deleting.minY + deleting.height / 3,
      width: deleting.width, height: deleting.height / 3)
    let deletingText = try textStyle("削除中", in: host, rect: deletingRow).rect
    let indicatorRegion = NSRect(
      x: deletingText.minX - deletingRow.height, y: deletingRow.minY,
      width: deletingRow.height, height: deletingRow.height)
    let indicator = try raster(in: host, rect: indicatorRegion)
    let indicatorBackground = try XCTUnwrap(indicator.pixels.first)
    let ink = indicator.pixels.indices.filter {
      let delta = indicator.pixels[$0] - indicatorBackground
      return (delta * delta).sum() > 0.01
    }
    XCTAssertGreaterThan(ink.count, 8, "The deleting row must display a progress mark")
    let center = CGPoint(x: CGFloat(indicator.width) / 2, y: CGFloat(indicator.height) / 2)
    for index in ink {
      let pixel = indicator.pixels[index]
      XCTAssertEqual(pixel.x, pixel.y, accuracy: 0.05)
      XCTAssertEqual(pixel.y, pixel.z, accuracy: 0.05)
      let dx = CGFloat(index % indicator.width) - center.x
      let dy = CGFloat(index / indicator.width) - center.y
      XCTAssertGreaterThan(dx * dx + dy * dy, 4)
    }
    _ = try textStyle(
      "Deleting session", in: host,
      rect: NSRect(
        x: deleting.minX, y: deleting.minY + deleting.height * 2 / 3,
        width: deleting.width, height: deleting.height / 3))
    let label = try XCTUnwrap(host.element("worktree:deleting")).label
    XCTAssertLessThan(
      try XCTUnwrap(label.range(of: "削除中")).lowerBound,
      try XCTUnwrap(label.range(of: "Deleting session")).lowerBound)
    let blank = try raster(
      in: host,
      rect: NSRect(
        x: empty.minX + 5, y: empty.midY, width: empty.width - 10, height: empty.height / 2 - 6))
    let background = try XCTUnwrap(blank.pixels.first)
    XCTAssertTrue(blank.pixels.allSatisfy { (($0 - background) * ($0 - background)).sum() < 0.02 })
    await model.stop()
  }

  @MainActor func test_B015_B016_B017_B019_B020_選択したworktreeの変更数だけをバッジに出す() async throws {
    // Given
    let model = AppModel()
    var repo = repository(["three", "hundred", "zero", "failed"])
    for (index, count) in [3, 100, 0].enumerated() {
      repo.branches.items[index].dirtyCount = UInt64(count)
    }
    repo.branches.items[3].dirtyCountError = "Count failed"
    model.repositories = [repo]
    let host = await ViewHarness(RightPanelView(expanded: .constant(true), model: model))
    defer { host.close() }
    // When / Then
    for (selection, expected) in [
      ("three", "3"), ("hundred", "99+"), ("zero", ""), ("failed", ""), ("", ""),
    ] {
      model.selectedWorktree = selection.isEmpty ? nil : "/acceptance/" + selection
      try await eventuallyView(host) { host.element("right-panel:sourceControl") != nil }
      if expected.isEmpty {
        let toggle = try frame("right-panel:sourceControl", in: host)
        try await eventuallyView(host) {
          (try? hasGrayBadge(in: host, rect: toggle.insetBy(dx: -10, dy: -10))) == false
        }
        XCTAssertFalse(try hasGrayBadge(in: host, rect: toggle.insetBy(dx: -10, dy: -10)))
      } else {
        try await eventuallyView(host) { host.element(expected) != nil }
        let badge = try frame(expected, in: host)
        let toggle = try frame("right-panel:sourceControl", in: host)
        XCTAssertGreaterThan(badge.midX, toggle.midX)
        XCTAssertLessThan(badge.midY, toggle.midY)
        XCTAssertTrue(try hasGrayBadge(in: host, rect: badge.insetBy(dx: -12, dy: -12)))
      }
    }
  }
  @MainActor private func axElements(in host: ViewHarness) throws -> [AXUIElement] {
    let app = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
    let windows = attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []
    let window = try XCTUnwrap(
      windows.first { attribute($0, kAXTitleAttribute) as? String == host.window.title })
    var visited: [AXUIElement] = []
    func collect(_ item: AXUIElement) {
      guard !visited.contains(where: { CFEqual($0, item) }) else { return }
      visited.append(item)
      for child in attribute(item, kAXChildrenAttribute) as? [AXUIElement] ?? [] { collect(child) }
    }
    collect(window)
    return visited
  }

  private func attribute(_ element: AXUIElement, _ key: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, key as CFString, &value) == .success else {
      return nil
    }
    return value
  }

  @MainActor private func assertWarningTooltip(
    _ expected: [String], in host: ViewHarness, rect: NSRect
  ) async throws {
    let view = try XCTUnwrap(host.window.contentView)
    let region = rect.integral.intersection(view.bounds)
    let actual = try raster(in: host, rect: region)
    var match: NSRect?
    for size in 10...20 {
      if let found = actual.find(try Self.reference("triangle-alert", size: size), color: nil) {
        match = found
        break
      }
    }
    let icon = try XCTUnwrap(match)
    let scale = CGFloat(actual.width) / region.width
    let local = NSRect(
      x: region.minX + icon.minX / scale, y: region.minY + icon.minY / scale,
      width: icon.width / scale, height: icon.height / scale)
    let help = try helpTexts(in: host, within: local).joined(separator: "\n")
    for text in expected {
      XCTAssertTrue(help.contains(text), "Warning help must contain: \(text)")
    }

    var screen = host.window.convertToScreen(view.convert(local, to: nil))
    screen.origin.y = try XCTUnwrap(NSScreen.screens.first).frame.height - screen.maxY
    let previous = try XCTUnwrap(CGEvent(source: nil)?.location)
    defer { CGWarpMouseCursorPosition(previous) }
    let app = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
    XCTAssertEqual(
      AXUIElementSetAttributeValue(app, kAXFrontmostAttribute as CFString, kCFBooleanTrue),
      .success)
    try await eventually {
      host.window.makeKeyAndOrderFront(nil)
      return NSApp.isActive && host.window.isKeyWindow
    }
    host.window.acceptsMouseMovedEvents = true
    let point = CGPoint(x: screen.midX, y: screen.midY)
    let visible = Set(NSApp.windows.filter(\.isVisible).map(ObjectIdentifier.init))
    for position in [CGPoint(x: point.x - 40, y: point.y), point] {
      XCTAssertEqual(CGWarpMouseCursorPosition(position), .success)
      let event = try XCTUnwrap(
        CGEvent(
          mouseEventSource: nil, mouseType: .mouseMoved,
          mouseCursorPosition: position, mouseButton: .left))
      event.post(tap: .cghidEventTap)
      let localEvent = try XCTUnwrap(
        NSEvent.mouseEvent(
          with: .mouseMoved,
          location: host.window.convertPoint(
            fromScreen: NSPoint(
              x: position.x, y: try XCTUnwrap(NSScreen.screens.first).frame.height - position.y)),
          modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
          windowNumber: host.window.windowNumber, context: nil, eventNumber: 1,
          clickCount: 0, pressure: 0))
      NSApp.postEvent(localEvent, atStart: false)
      try await Task.sleep(for: .milliseconds(100))
    }
    try await eventually {
      NSApp.windows.contains {
        $0 !== host.window && $0.isVisible && !visible.contains(ObjectIdentifier($0))
      }
    }
    let tooltip = try XCTUnwrap(
      NSApp.windows.first {
        $0 !== host.window && $0.isVisible && !visible.contains(ObjectIdentifier($0))
      })
    let content = try XCTUnwrap(tooltip.contentView)
    let bitmap = try XCTUnwrap(content.bitmapImageRepForCachingDisplay(in: content.bounds))
    content.cacheDisplay(in: content.bounds, to: bitmap)
    let request = VNRecognizeTextRequest()
    request.recognitionLanguages = ["en-US"]
    request.usesLanguageCorrection = false
    try VNImageRequestHandler(cgImage: XCTUnwrap(bitmap.cgImage)).perform([request])
    let text = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
      .joined(separator: " ")
    for value in expected {
      XCTAssertTrue(text.contains(value), "Displayed tooltip: \(text); expected: \(value)")
    }
    XCTAssertEqual(CGWarpMouseCursorPosition(previous), .success)
    let exitEvent = try XCTUnwrap(
      CGEvent(
        mouseEventSource: nil, mouseType: .mouseMoved,
        mouseCursorPosition: previous, mouseButton: .left))
    exitEvent.post(tap: .cghidEventTap)
    try await eventually { !tooltip.isVisible }
  }

  @MainActor private func helpTexts(in host: ViewHarness, within rect: NSRect? = nil) throws
    -> [String]
  {
    try axElements(in: host).compactMap { element in
      if let rect {
        guard let value = attribute(element, "AXFrame") else { return nil }
        var screen = CGRect.zero
        guard AXValueGetValue(value as! AXValue, .cgRect, &screen) else { return nil }
        screen.origin.y = try XCTUnwrap(NSScreen.screens.first).frame.height - screen.maxY
        let view = try XCTUnwrap(host.window.contentView)
        let frame = view.convert(host.window.convertFromScreen(screen), from: nil)
        guard rect.intersects(frame) else { return nil }
      }
      return attribute(element, kAXHelpAttribute) as? String
    }
  }

  @MainActor private func raster(in host: ViewHarness, rect: NSRect) throws -> SidebarIconRaster {
    host.window.appearance = NSAppearance(named: .aqua)
    let view = try XCTUnwrap(host.window.contentView)
    view.layoutSubtreeIfNeeded()
    host.window.displayIfNeeded()
    let region = rect.integral.intersection(view.bounds)
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: region))
    view.cacheDisplay(in: region, to: bitmap)
    return try SidebarIconRaster(bitmap: bitmap)
  }

  @MainActor private func containsIcon(_ name: String, in host: ViewHarness, rect: NSRect) throws
    -> Bool
  {
    let actual = try raster(in: host, rect: rect)
    for size in 10...20 {
      if actual.find(try Self.reference(name, size: size), color: nil) != nil { return true }
    }
    return false
  }

  @MainActor private func iconCount(_ name: String, in host: ViewHarness, rect: NSRect) throws
    -> Int
  {
    let actual = try raster(in: host, rect: rect)
    for size in 10...20 {
      let expected = try Self.reference(name, size: size)
      if let first = actual.find(expected, color: nil) {
        var pixels = actual.pixels
        for y in Int(first.minY)..<Int(first.maxY) {
          for x in Int(first.minX)..<Int(first.maxX) { pixels[y * actual.width + x] = pixels[0] }
        }
        let masked = SidebarIconRaster(width: actual.width, height: actual.height, pixels: pixels)
        return masked.find(expected, color: nil) == nil ? 1 : 2
      }
    }
    return 0
  }

  @MainActor private func referenceRaster(_ view: NSView) throws -> SidebarIconRaster {
    let window = NSWindow(
      contentRect: NSRect(origin: .zero, size: view.fittingSize),
      styleMask: [.borderless], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    defer { window.close() }
    window.contentView = view
    view.layoutSubtreeIfNeeded()
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
    view.cacheDisplay(in: view.bounds, to: bitmap)
    return try SidebarIconRaster(bitmap: bitmap)
  }

  @MainActor private func hasGrayBadge(in host: ViewHarness, rect: NSRect) throws -> Bool {
    let actual = try raster(in: host, rect: rect)
    for diameter in 12...28 {
      let side = CGFloat(diameter)
      let view = NSHostingView(
        rootView: Circle().fill(.black)
          .frame(width: side, height: side).padding(2).background(.white))
      view.setFrameSize(view.fittingSize)
      view.layoutSubtreeIfNeeded()
      let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
      view.cacheDisplay(in: view.bounds, to: bitmap)
      if actual.find(try SidebarIconRaster(bitmap: bitmap), color: .gray, ignoringCenter: true)
        != nil
      {
        return true
      }
    }
    return false
  }

  @MainActor private func textStyle(_ text: String, in host: ViewHarness, rect: NSRect) throws
    -> (size: Int, color: SIMD3<Double>, rect: NSRect)
  {
    let actual = try raster(in: host, rect: rect)
    for size in 8...24 {
      let view = NSHostingView(
        rootView: Text(text).font(.system(size: CGFloat(size)))
          .foregroundStyle(.black).padding(2).background(.white))
      let reference = try referenceRaster(view)
      if let match = actual.find(reference, color: nil) {
        let ink = reference.pixels.indices.filter { reference.pixels[$0].sum() < 0.15 }
        XCTAssertFalse(ink.isEmpty)
        let color =
          ink.reduce(SIMD3<Double>.zero) { result, index in
            result
              + actual.pixels[
                (Int(match.minY) + index / reference.width) * actual.width
                  + Int(match.minX) + index % reference.width]
          } / Double(max(ink.count, 1))
        let region = rect.integral.intersection(try XCTUnwrap(host.window.contentView).bounds)
        let scale = CGFloat(actual.width) / region.width
        return (
          size, color,
          NSRect(
            x: region.minX + match.minX / scale, y: region.minY + match.minY / scale,
            width: match.width / scale, height: match.height / scale)
        )
      }
    }
    XCTFail("Rendered title not found: \(text)")
    throw NSError(domain: "SidebarAcceptanceTests", code: 1)
  }

  @MainActor private static func reference(_ name: String, size: Int, scale: CGFloat? = nil)
    throws
    -> SidebarIconRaster
  {
    let svg = try XCTUnwrap(LucideAcceptanceReference.svg[name])
    let image = try XCTUnwrap(NSImage(data: Data(svg.utf8)))
    return try SidebarIconRaster(image: image, size: size, scale: scale)
  }

  @MainActor private func frame(_ name: String, in host: ViewHarness) throws -> NSRect {
    let keys = [
      kAXIdentifierAttribute, kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute,
    ]
    let element = try XCTUnwrap(
      try axElements(in: host).first { element in
        keys.contains { attribute(element, $0) as? String == name }
      }, name)
    return try frame(element, in: host)
  }

  @MainActor private func frame(_ element: AXUIElement, in host: ViewHarness) throws -> NSRect {
    let value = try XCTUnwrap(attribute(element, "AXFrame"))
    var screen = CGRect.zero
    XCTAssertTrue(AXValueGetValue(value as! AXValue, .cgRect, &screen))
    screen.origin.y = try XCTUnwrap(NSScreen.screens.first).frame.height - screen.maxY
    let view = try XCTUnwrap(host.window.contentView)
    return view.convert(host.window.convertFromScreen(screen), from: nil)
  }

  @MainActor private func assertIcon(
    _ name: String, in host: ViewHarness, element: String? = nil, rect: NSRect? = nil,
    sizes: [Int] = Array(10...20), color: Color? = nil,
    file: StaticString = #filePath, line: UInt = #line
  ) async throws {
    if let element { try await eventuallyView(host, { host.element(element) != nil }) }
    let view = try XCTUnwrap(host.window.contentView)
    host.window.appearance = NSAppearance(named: .aqua)
    view.layoutSubtreeIfNeeded()
    host.window.displayIfNeeded()
    let bounds = try rect ?? element.map { try frame($0, in: host) } ?? view.bounds
    // Settings のタブはタイトルバーに配置され、contentView の外側にある。
    let captureView = try view.bounds.contains(bounds) ? view : XCTUnwrap(view.superview)
    let region = captureView.convert(bounds, from: view).integral.intersection(captureView.bounds)
    let bitmap = try XCTUnwrap(
      captureView.bitmapImageRepForCachingDisplay(in: region),
      "\(element ?? name): bounds=\(bounds), capture=\(captureView.bounds), region=\(region)",
      file: file, line: line)
    captureView.cacheDisplay(in: region, to: bitmap)
    let actual = try SidebarIconRaster(bitmap: bitmap)
    var match: NSRect?
    for size in sizes {
      let expected = try Self.reference(
        name, size: size, scale: CGFloat(bitmap.pixelsWide) / region.width)
      if let found = actual.find(expected, color: color) {
        match = found
        break
      }
    }
    if match == nil {
      let image = NSImage(size: bitmap.size)
      image.addRepresentation(bitmap)
      let attachment = XCTAttachment(image: image)
      attachment.name = "\(name)-\(element ?? "view")"
      attachment.lifetime = .keepAlways
      add(attachment)
    }
    XCTAssertNotNil(
      match, "\(name), \(element ?? "view"), sizes=\(sizes)", file: file, line: line)
  }

}

private struct SidebarIconRaster {
  let width: Int
  let height: Int
  let pixels: [SIMD3<Double>]

  init(width: Int, height: Int, pixels: [SIMD3<Double>]) {
    self.width = width
    self.height = height
    self.pixels = pixels
  }

  /// 参照画像を、実画面と同じ描画経路（NSHostingView を cacheDisplay で写す）で作る。
  /// 描画経路が違うと、8 ポイントのような小さい大きさでは同じ SVG でも画素が合わない
  @MainActor init(image: NSImage, size: Int, scale: CGFloat? = nil) throws {
    let side = CGFloat(size)
    let view = NSHostingView(
      rootView: Image(nsImage: image).resizable().renderingMode(.template)
        .foregroundStyle(.black).frame(width: side, height: side).background(.white))
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: side, height: side), styleMask: [.borderless],
      backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    defer { window.close() }
    window.contentView = view
    view.layoutSubtreeIfNeeded()
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
    view.cacheDisplay(in: view.bounds, to: bitmap)
    if let scale {
      XCTAssertEqual(CGFloat(bitmap.pixelsWide), side * scale, "参照画像と実画面の倍率が違う")
    }
    try self.init(bitmap: bitmap)
  }

  init(bitmap: NSBitmapImageRep) throws {
    // 画面を写した画像は表示先のディスプレイの色空間である。色を比べる前に sRGB に直す
    let bitmap = try XCTUnwrap(bitmap.converting(to: .sRGB, renderingIntent: .default))
    let width = bitmap.pixelsWide
    let height = bitmap.pixelsHigh
    self.width = width
    self.height = height
    pixels = try (0..<height).flatMap { y in
      try (0..<width).map { x in
        let color = try XCTUnwrap(bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB))
        let alpha = Double(color.alphaComponent)
        return SIMD3(
          Double(color.redComponent), Double(color.greenComponent), Double(color.blueComponent))
          * alpha + (1 - alpha)
      }
    }
  }

  @MainActor func find(_ reference: Self, color: Color?, ignoringCenter: Bool = false) -> NSRect? {
    guard width >= reference.width, height >= reference.height else { return nil }
    let weights = reference.pixels.map { 1 - ($0.x + $0.y + $0.z) / 3 }
    let energy = weights.reduce(0) { $0 + $1 * $1 }
    guard energy > 1 else { return nil }
    let required = color.flatMap { NSColor($0).usingColorSpace(.sRGB) }.map {
      SIMD3(Double($0.redComponent), Double($0.greenComponent), Double($0.blueComponent))
    }
    func ignored(_ index: Int) -> Bool {
      let dx = Double(index % reference.width) - Double(reference.width) / 2
      let dy = Double(index / reference.width) - Double(reference.height) / 2
      return ignoringCenter
        && (weights[index] < 0.01
          || dx * dx + dy * dy < pow(Double(reference.width) * 0.3, 2))
    }
    let samples = weights.indices.filter { weights[$0] > 0.5 && !ignored($0) }
    let sampleEnergy = samples.reduce(0) { $0 + weights[$1] * weights[$1] }
    for y in 0...(height - reference.height) {
      for x in 0...(width - reference.width) {
        let background = pixels[y * width + x]
        var contrast = SIMD3<Double>.zero
        for index in samples {
          contrast +=
            (pixels[(y + index / reference.width) * width + x + index % reference.width]
              - background) * weights[index]
        }
        contrast /= sampleEnergy
        let foreground = background + contrast
        let magnitude = (contrast * contrast).sum()
        guard magnitude > 0.12 else { continue }
        if let required, ((foreground - required) * (foreground - required)).sum() > 0.12 {
          continue
        }
        var error = 0.0
        let limit = energy * magnitude * 0.10
        for index in weights.indices where !ignored(index) {
          let actual = pixels[(y + index / reference.width) * width + x + index % reference.width]
          let delta = actual - (background + contrast * weights[index])
          error += (delta * delta).sum()
          if error > limit { break }
        }
        if error <= limit {
          return NSRect(x: x, y: y, width: reference.width, height: reference.height)
        }
      }
    }
    return nil
  }

}

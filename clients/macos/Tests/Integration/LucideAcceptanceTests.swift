import AppKit
import Bonsplit
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class LucideAcceptanceTests: XCTestCase {
  @MainActor private var menuImageInspected: XCTestExpectation?

  @MainActor func test_B001_B005_サイドバーの操作とカードが指定された形と色になる() async throws {
    // Given
    let model = AppModel()
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.path = "/icons"
    for state in ["main", "open", "draft", "merged", "closed"] {
      var branch = Releash_Client_V1_WorkspaceBranch()
      branch.name = state
      branch.worktreePath = "/icons/" + state
      branch.isMainWorktree = state == "main"
      branch.isMerged = state == "merged"
      if state != "main" { branch.prState = state }
      repository.branches.items.append(branch)
      var worktree = Releash_Client_V1_WorkspaceWorktreeList()
      worktree.path = branch.worktreePath
      if state == "main" {
        worktree.executions = [false, true].map { workflow in
          var execution = Releash_Client_V1_WorktreeExecutionSummary()
          execution.id = workflow ? "workflow" : "session"
          execution.title = execution.id
          execution.isWorkflow = workflow
          execution.status = "active"
          return execution
        }
      }
      repository.worktrees.items.append(worktree)
    }
    model.repositories = [repository]
    // When
    let host = await ViewHarness(
      SidebarView(model: model, creating: .constant(false)),
      size: NSSize(width: 340, height: 1000))
    defer { host.close() }
    // Then
    for (label, icon) in [
      ("Repositoryを追加", "folder-plus"), ("再読み込み", "refresh-cw"), ("Worktreeを作成", "plus"),
    ] { try await assertIcon(icon, in: host, element: label) }
    for (state, icon) in [
      ("main", "house"), ("open", "git-pull-request"), ("draft", "git-pull-request-draft"),
      ("merged", "git-merge"), ("closed", "git-pull-request-closed"),
    ] {
      try await assertIcon(icon, in: host, element: "worktree:" + state)
      if state != "main" {
        try await assertIcon("git-branch", in: host, element: "worktree:" + state)
      }
    }
    try await assertIcon("circle-check", in: host, element: "worktree:merged")
    try await assertIcon("bot", in: host, element: "worktree:main", color: .blue)
    try await assertIcon("waypoints", in: host, element: "worktree:main")
  }

  @MainActor func test_B002_B004_Nodeの全種別が8ポイントの状態色のアイコンになる() async throws {
    // Given
    let model = AppModel()
    for (status, color) in [
      (Releash_Client_V1_WorkspaceStatusClassification.Value.attention, Color.yellow),
      (.active, .blue), (.idle, .green),
    ] {
      for (kind, icon) in [
        ("session", "bot"), ("command", "terminal"),
        ("sequence", "waypoints"), ("fanout", "git-fork"),
      ] {
        var item = Releash_Client_V1_WorkspaceTreeItem()
        switch kind {
        case "sequence":
          item.sequence.id = kind
          item.sequence.title = kind
          item.sequence.status.value = status
        case "fanout":
          item.fanout.id = kind
          item.fanout.title = kind
          item.fanout.status.value = status
        default:
          item.node.id = kind
          item.node.title = kind
          item.node.contentKind.value = kind == "command" ? .command : .session
          item.node.status.value = status
        }
        var repository = Releash_Client_V1_WorkspaceRepositoryList()
        repository.path = "/icons"
        repository.worktrees.items = [
          .with {
            $0.path = "/icons"
            $0.snapshot.nodes.items = [item]
          }
        ]
        model.repositories = [repository]
        // When
        let host = await ViewHarness(RootListView(model: model, path: "/icons"))
        // Then
        try await assertIcon(
          icon, in: host, element: "root:" + kind, sizes: [8],
          color: color, onlyColoredMark: true)
        host.close()
      }
    }
  }

  @MainActor func test_B003_B010_行を選ぶと詳細にも8ポイントの状態色の種別が出る() async throws {
    // Given
    let http = TestHTTPClient()
    var worktree = Releash_Client_V1_WorkspaceWorktreeList()
    worktree.path = "/icons"
    for command in [false, true] {
      let id = command ? "command" : "session"
      var item = Releash_Client_V1_WorkspaceTreeItem()
      item.node.id = id
      item.node.title = id
      item.node.contentKind.value = command ? .command : .session
      item.node.status.value = .active
      worktree.snapshot.nodes.items.append(item)
      var detail = Releash_Client_V1_WorkspaceNodeDetailDto()
      detail.id = id
      detail.title = "Selected " + id
      detail.classification.value = .active
      if command { detail.content.command = .init() } else { detail.content.session = .init() }
      http.state.withLock { $0.nodeDetails[id] = detail }
    }
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.path = "/icons"
    repository.worktrees.items = [worktree]
    http.state.withLock { $0.repositories = [repository] }
    let model = try await testModel(http)
    let host = await ViewHarness(RootListView(model: model, path: "/icons"))
    defer { host.close() }
    for (id, icon) in [("session", "bot"), ("command", "terminal")] {
      // When
      try await eventuallyView(host) { host.element("root:" + id) != nil }
      try await host.selectRow("root:" + id)
      try await eventuallyView(host) { host.element("Selected " + id) != nil }
      // Then: limit the search to the selected detail header, excluding the tree rows.
      let title = try frame("Selected " + id, in: host)
      let header = NSRect(x: 0, y: title.minY - 4, width: title.maxX, height: title.height + 8)
      try await assertIcon(
        icon, in: host, rect: header, sizes: [8], color: .blue, onlyColoredMark: true)
    }
    await model.stop()
  }

  @MainActor func test_B006_B007_中央タブは全種別が丸でSessionだけ状態色になる() async throws {
    // Given
    let model = AppModel()
    let workspace = model.panes
    try workspace.restore(.empty())
    let pane = try XCTUnwrap(workspace.controller.allPaneIds.first)
    workspace.open(.terminal, in: pane)
    workspace.openFile("example.txt", base: "head")
    workspace.openSession(id: "agent", nodeID: "session", title: "Session")
    workspace.openNode(.command, id: "command", title: "Command")
    let session = try XCTUnwrap(
      workspace.controller.allTabIds.first {
        workspace.tab($0)?.kind == .session
      })
    var snapshot = Releash_Client_V1_SessionTabSnapshot()
    snapshot.node.title = "Active Session"
    snapshot.node.classification.value = .active
    workspace.updateSession(session, snapshot: snapshot)
    // When
    let host = await ViewHarness(
      WorkspacePaneView(model: model), size: NSSize(width: 1000, height: 300))
    defer { host.close() }
    // Then
    for title in ["Terminal 1", "example.txt", "Active Session", "Command"] {
      try await assertTabDot(title, in: host, active: title == "Active Session")
    }
  }

  @MainActor func test_B009_配布アプリが公式のLucideライセンス全文を含む() throws {
    // Given
    let bundle = Bundle(for: AppModel.self)
    // When
    let url = try XCTUnwrap(bundle.url(forResource: "Lucide-LICENSE", withExtension: nil))
    let actual = try String(contentsOf: url, encoding: .utf8)
    // Then
    XCTAssertEqual(actual, LucideAcceptanceReference.license)
  }

  @MainActor func test_B012_右カラムの切り替えと開閉に指定アイコンが出る() async throws {
    // Given
    let model = AppModel()
    model.selectedWorktree = "/icons"
    // When
    let host = await ViewHarness(RightPanelView(expanded: .constant(true), model: model))
    defer { host.close() }
    // Then
    for (label, icon) in [
      ("right-panel:roots", "list"), ("right-panel:files", "file"),
      ("right-panel:sourceControl", "git-branch"), ("right-panel:threads", "message-square"),
      ("右パネルを閉じる", "panel-right"),
    ] { try await assertIcon(icon, in: host, element: label) }
  }

  @MainActor func test_B013_Nodeのメニューと過去の試行に指定アイコンが出る() async throws {
    // Given
    let model = AppModel()
    model.layoutLoaded = true
    var item = Releash_Client_V1_WorkspaceTreeItem()
    item.node.id = "current"
    item.node.title = "Current"
    item.node.pastAttempts.items = [.with { $0.node.id = "past" }]
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.worktrees.items = [
      .with {
        $0.path = "/icons"
        $0.snapshot.nodes.items = [item]
      }
    ]
    model.repositories = [repository]
    // When
    let host = await ViewHarness(RootListView(model: model, path: "/icons"))
    defer { host.close() }
    // Then
    for (label, icon) in [("その他", "ellipsis"), ("開始", "plus"), ("過去の試行", "history")] {
      try await assertIcon(icon, in: host, element: label)
    }
  }

  @MainActor func test_B014_解決済みThreadに指定アイコンが出る() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.reviewThreads["/icons"] = .with {
        $0.items = [testReviewThread("resolved", resolved: true)]
      }
    }
    let model = try await testModel(http)
    let host = await ViewHarness(ThreadPanelView(model: model, worktree: "/icons"))
    defer { host.close() }
    // When
    try await eventuallyView(host) { host.element("解決済み 1") != nil }
    try await host.press("解決済み 1")
    // Then
    try await assertIcon("circle-check", in: host, element: "thread-resolved:resolved")
    await model.stop()
  }

  @MainActor func test_B015_開くメニューとTerminal項目に指定アイコンが出る() async throws {
    // Given
    let model = AppModel()
    model.layoutLoaded = true
    try model.panes.restore(.empty())
    let pane = try XCTUnwrap(model.panes.controller.allPaneIds.first)
    let host = await ViewHarness(PaneOpenMenu(model: model, pane: pane))
    defer { host.close() }
    try await assertIcon("plus", in: host, element: "開く")
    // When / Then
    let inspected = expectation(description: "Terminal menu image inspected")
    menuImageInspected = inspected
    NotificationCenter.default.addObserver(
      self, selector: #selector(inspectTerminalMenu(_:)),
      name: NSMenu.didBeginTrackingNotification, object: nil)
    defer {
      NotificationCenter.default.removeObserver(
        self, name: NSMenu.didBeginTrackingNotification, object: nil)
      menuImageInspected = nil
    }
    try await host.menu("開く", selecting: nil) { titles in
      XCTAssertTrue(titles.contains("Terminal"))
    }
    await fulfillment(of: [inspected], timeout: 1)
  }

  @MainActor func test_B016_ファイルのDiff操作に指定アイコンが出る() async throws {
    // Given
    let model = AppModel()
    try model.panes.restore(.empty())
    model.panes.openFile("example.txt", base: "head")
    let view = FileTabPane(
      model: model, tabID: try XCTUnwrap(model.panes.controller.allTabIds.first),
      path: "example.txt", worktree: "/icons", value: .init())
    view.setPresentation(mode: "diff")
    // When
    let host = await ViewHarness(view.toolbar, size: NSSize(width: 700, height: 60))
    defer { host.close() }
    // Then
    for (label, icon) in [
      ("file-mode", "diff"), ("file-open-editor", "square-arrow-out-up-right"),
      ("file-full", "unfold-vertical"), ("HEAD と比較", "git-commit-horizontal"),
      ("base と比較", "git-branch"), ("上下に並べる", "rows-2"), ("左右に並べる", "columns-2"),
    ] { try await assertIcon(icon, in: host, element: label) }
  }

  @MainActor func test_B005_B017_B018_フッターのproviderと警告と版の状態を描く() async throws {
    // Given
    let model = AppModel()
    model.daemonInfo = .init()
    model.providerUsage = ["claude", "codex"].map { name in .with { $0.provider = name } }
    model.hookHealth.readErrors = ["Unavailable"]
    model.hookHealth.warnings.items = [
      .with {
        $0.provider.value = .claude
        $0.reason = "Warning"
      }
    ]
    for compatibility in ["compatible", "server_older"] {
      model.releaseCompatibility = compatibility
      // When
      let host = await ViewHarness(FooterView(model: model), size: NSSize(width: 1000, height: 40))
      // Then
      try await assertIcon("sparkles", in: host, element: "footer-provider:claude")
      try await assertIcon("terminal", in: host, element: "footer-provider:codex")
      try await assertIcon("triangle-alert", in: host, element: "footer-provider:claude")
      try await assertIcon(
        "triangle-alert", in: host, element: "hookの状態を確認できません", color: .yellow)
      try await assertIcon(
        compatibility == "compatible" ? "server" : "triangle-alert", in: host,
        element: "footer-server")
      host.close()
    }
  }

  @MainActor func test_B019_接続できない画面に警告アイコンが出る() async throws {
    // Given
    let model = AppModel()
    model.connectionFailure = "Connection unavailable"
    // When
    let host = await ViewHarness(RootView(model: model))
    defer { host.close() }
    // Then
    try await assertIcon(
      "triangle-alert", in: host, element: "サーバに接続できません", sizes: Array(24...64))
  }

  @MainActor func test_B020_Settingsのタブと未選択のRepositoriesに指定アイコンが出る() async throws {
    // Given
    let model = AppModel()
    let host = await ViewHarness(SettingsView(model: model))
    defer { host.close() }
    // When
    try await host.press("Repositories")
    // Then
    for (label, icon) in [
      ("General", "settings"), ("Repositories", "folder"), ("Providers", "terminal"),
      ("Workflows", "waypoints"), ("System", "monitor"),
    ] { try await assertIcon(icon, in: host, element: label, sizes: Array(12...32)) }
    try await assertIcon(
      "folder", in: host, element: "Repositoryを選択してください", sizes: Array(24...64))
  }

}

extension LucideAcceptanceTests {
  @MainActor private func assertTabDot(_ title: String, in host: ViewHarness, active: Bool)
    async throws
  {
    try await eventuallyView(host) { host.element(title) != nil }
    let view = try XCTUnwrap(host.window.contentView)
    host.window.appearance = NSAppearance(named: .aqua)
    view.layoutSubtreeIfNeeded()
    host.window.displayIfNeeded()
    let tab = try frame(title, in: host)
    let region = NSRect(x: tab.minX, y: tab.minY, width: 44, height: tab.height)
      .integral.intersection(view.bounds)
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: region))
    view.cacheDisplay(in: region, to: bitmap)
    let actual = try IconRaster(bitmap: bitmap)
    var match: NSRect?
    for diameter in 8...18 {
      let renderer = ImageRenderer(
        content: Circle().fill(.black)
          .frame(width: CGFloat(diameter), height: CGFloat(diameter)).padding(2).background(.white))
      renderer.scale = CGFloat(bitmap.pixelsWide) / region.width
      let expected = try IconRaster(bitmap: NSBitmapImageRep(cgImage: XCTUnwrap(renderer.cgImage)))
      if let found = actual.find(expected, color: active ? .blue : nil) {
        match = found
        break
      }
    }
    if match == nil {
      let image = NSImage(size: bitmap.size)
      image.addRepresentation(bitmap)
      let attachment = XCTAttachment(image: image)
      attachment.name = "tab-\(title)"
      attachment.lifetime = .keepAlways
      add(attachment)
    }
    XCTAssertNotNil(match, "B-006/B-007: \(title) のアイコンが \(active ? "青い" : "無彩色の") ● である")
    if !active {
      XCTAssertFalse(actual.hasColoredPixels(outside: .zero), "B-007: \(title) に状態の色を付けない")
    }
  }

  @MainActor @objc private func inspectTerminalMenu(_ notification: Notification) {
    guard let menu = notification.object as? NSMenu,
      let item = menu.items.first(where: { $0.title == "Terminal" })
    else { return }
    do {
      let image = try XCTUnwrap(item.image)
      let actual = try IconRaster(image: image, size: 24)
      let expected = try Self.reference("terminal", size: 24)
      XCTAssertNotNil(actual.find(expected, color: nil), "B-015: Terminal menu image")
      menuImageInspected?.fulfill()
    } catch { XCTFail("\(error)") }
  }

  @MainActor private static func reference(_ name: String, size: Int, scale: CGFloat = 2) throws
    -> IconRaster
  {
    let svg = try XCTUnwrap(LucideAcceptanceReference.svg[name])
    let image = try XCTUnwrap(NSImage(data: Data(svg.utf8)))
    return try IconRaster(image: image, size: size, scale: scale)
  }

  @MainActor private func frame(_ name: String, in host: ViewHarness) throws -> NSRect {
    func attribute(_ element: AXUIElement, _ key: String) -> CFTypeRef? {
      var value: CFTypeRef?
      guard AXUIElementCopyAttributeValue(element, key as CFString, &value) == .success else {
        return nil
      }
      return value
    }
    func find(_ element: AXUIElement) -> AXUIElement? {
      let keys = [
        kAXIdentifierAttribute, kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute,
      ]
      if keys.contains(where: { attribute(element, $0) as? String == name }) { return element }
      return (attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []).lazy
        .compactMap(find).first
    }
    let app = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
    let window = try XCTUnwrap(
      (attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []).first {
        attribute($0, kAXTitleAttribute) as? String == host.window.title
      }, name)
    let value = try XCTUnwrap(attribute(try XCTUnwrap(find(window), name), "AXFrame"), name)
    var screen = CGRect.zero
    XCTAssertTrue(AXValueGetValue(value as! AXValue, .cgRect, &screen), name)
    // AX の座標は主画面の左上が原点である。AppKit の画面座標（左下が原点）に直す
    screen.origin.y = try XCTUnwrap(NSScreen.screens.first).frame.height - screen.maxY
    let view = try XCTUnwrap(host.window.contentView)
    return view.convert(host.window.convertFromScreen(screen), from: nil)
  }

  @MainActor private func assertIcon(
    _ name: String, in host: ViewHarness, element: String? = nil, rect: NSRect? = nil,
    sizes: [Int] = Array(10...20), color: Color? = nil, onlyColoredMark: Bool = false,
    file: StaticString = #filePath, line: UInt = #line
  ) async throws {
    if let element { try await eventuallyView(host, { host.element(element) != nil }) }
    let view = try XCTUnwrap(host.window.contentView)
    host.window.appearance = NSAppearance(named: .aqua)
    view.layoutSubtreeIfNeeded()
    host.window.displayIfNeeded()
    let region = try (rect ?? element.map { try frame($0, in: host) } ?? view.bounds)
      .integral.intersection(view.bounds)
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: region))
    view.cacheDisplay(in: region, to: bitmap)
    let actual = try IconRaster(bitmap: bitmap)
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
      match, "B-008: \(name), \(element ?? "view"), sizes=\(sizes)", file: file, line: line)
    if onlyColoredMark, let match {
      XCTAssertFalse(
        actual.hasColoredPixels(outside: match),
        "状態色の種別アイコン以外に状態の丸が残っている", file: file, line: line)
    }
  }

}

private struct IconRaster {
  let width: Int
  let height: Int
  let pixels: [SIMD3<Double>]

  @MainActor init(image: NSImage, size: Int, scale: CGFloat = 2) throws {
    let renderer = ImageRenderer(
      content: Image(nsImage: image).resizable().renderingMode(.template)
        .foregroundStyle(.black).frame(width: CGFloat(size), height: CGFloat(size))
        .background(.white))
    renderer.scale = scale
    try self.init(bitmap: NSBitmapImageRep(cgImage: XCTUnwrap(renderer.cgImage)))
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

  @MainActor func find(_ reference: Self, color: Color?) -> NSRect? {
    guard width >= reference.width, height >= reference.height else { return nil }
    let weights = reference.pixels.map { 1 - ($0.x + $0.y + $0.z) / 3 }
    let energy = weights.reduce(0) { $0 + $1 * $1 }
    guard energy > 1 else { return nil }
    let required = color.flatMap { NSColor($0).usingColorSpace(.sRGB) }.map {
      SIMD3(Double($0.redComponent), Double($0.greenComponent), Double($0.blueComponent))
    }
    let samples = weights.indices.filter { weights[$0] > 0.5 }
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
        for index in weights.indices {
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

  func hasColoredPixels(outside rect: NSRect) -> Bool {
    pixels.enumerated().contains { index, pixel in
      !rect.insetBy(dx: -1, dy: -1).contains(NSPoint(x: index % width, y: index / width))
        && max(pixel.x, pixel.y, pixel.z) - min(pixel.x, pixel.y, pixel.z) > 0.3
    }
  }
}

import AppKit
import ApplicationServices
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class SettingsAcceptanceTests: XCTestCase {
  @MainActor func test_設定画面_B001_B002_設定の自然サイズと左ナビゲーションから右ページへ切り替える() async throws {
    // Given
    let model = try await testModel(TestHTTPClient())
    let view = NSHostingView(rootView: SettingsView(model: model))
    // When
    let host = await ViewHarness(SettingsView(model: model), size: view.fittingSize)
    defer { host.close() }
    // Then
    XCTAssertEqual(host.window.contentLayoutRect.size, NSSize(width: 1024, height: 800))
    let titles = ["General", "Repositories", "Providers", "Workflows", "System"]
    try await eventuallyView(host) { host.element("settings-terminal-command") != nil }
    let navigation = try titles.map { try frame($0, host) }
    for (upper, lower) in zip(navigation, navigation.dropFirst()) {
      XCTAssertLessThan(upper.maxY, lower.midY)
      XCTAssertEqual(upper.minX, lower.minX, accuracy: 2)
    }
    for title in titles { try assertIconPresent(title, host) }
    XCTAssertLessThan(
      try frame("General", host).maxX, try frame("settings-terminal-command", host).minX)
    // When
    try await host.press("System")
    // Then
    try await eventuallyView(host) { host.element("settings-auto-launch") != nil }
    XCTAssertLessThan(try frame("System", host).maxX, try frame("settings-auto-launch", host).minX)
    XCTAssertFalse(host.window.title.contains("System"))
    let contentTop = host.window.convertToScreen(host.window.contentLayoutRect).maxY
    for item in try elements("System", host) {
      XCTAssertGreaterThanOrEqual(try screenFrame(item).minY, screenHeight - contentTop)
    }
    await model.stop()
  }

  @MainActor func test_一般設定_B003_B015_Generalのまとまりと左右の行と説明() async throws {
    // Given
    let model = try await testModel(TestHTTPClient())
    let host = await ViewHarness(SettingsView(model: model), size: NSSize(width: 1024, height: 800))
    defer { host.close() }
    // When
    try await host.press("General")
    // Then
    try await eventuallyView(host) { host.element("settings-terminal-command")?.enabled == true }
    try assertGroup(
      ["settings-external-editor", "settings-diff-base"],
      excluding: ["settings-terminal-command", "settings-approval-automation"], host)
    for heading in ["エディタ", "Terminal", "Workflow"] { XCTAssertNotNil(host.element(heading)) }
    try assertGroup(
      ["settings-terminal-command"],
      excluding: ["settings-external-editor", "settings-approval-automation"], host)
    try assertGroup(
      ["settings-approval-automation"],
      excluding: ["settings-external-editor", "settings-terminal-command"], host)
    for (label, control) in [
      ("外部エディタ", "settings-external-editor"), ("Diff base の既定", "settings-diff-base"),
      ("Terminal の起動コマンド", "settings-terminal-command"), ("承認の自動化", "settings-approval-automation"),
    ] { try assertRow(label, control: control, host) }
    try assertDescription("Diff base の既定", "ファイルタブで差分を開いたときに、最初に比べる相手です。", host)
    try assertDescription(
      "Terminal の起動コマンド", "新しい Terminal のタブで shell が起動した直後に、このコマンドを入力して実行します。空なら何もしません。", host)
    try assertDescription("承認の自動化", "承認を求める Node を、人の確認なしに完了させます。", host)
    await model.stop()
  }

  @MainActor func test_リポジトリ設定_B004_B008_B009_B017_Repository一覧から詳細とNotionのLabelsを表示して戻る()
    async throws
  {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.repositories = [
        .with {
          $0.path = "/work"
          $0.name = "releash"
        },
        .with {
          $0.path = "/articles"
          $0.name = "articles"
        },
      ]
      $0.notionConfigs["/work"] = .with {
        $0.value.apiToken = "test-token"
        $0.value.databaseID = "test-database"
      }
      $0.notionValidation.status.value = .configured
      $0.notionValidation.properties.items = [
        .with {
          $0.name = "Name"
          $0.propertyType = "title"
        },
        .with {
          $0.name = "Status"
          $0.propertyType = "status"
        },
      ]
    }
    let model = try await testModel(http)
    let host = await ViewHarness(SettingsView(model: model), size: NSSize(width: 1024, height: 800))
    defer { host.close() }
    // When
    try await host.press("Repositories")
    // Then
    try await eventuallyView(host) { host.texts().contains(where: { $0.contains("releash") }) }
    for (name, path, otherPath) in [
      ("releash", "/work", "/articles"), ("articles", "/articles", "/work"),
    ] {
      let row = try repositoryRow(name, path: path, host)
      XCTAssertFalse(texts(in: row).contains { $0.contains(otherPath) })
    }
    XCTAssertNil(host.element("settings-base-branch"))
    // When
    try await selectRepository("releash", host)
    // Then
    try await eventuallyView(host) { host.element("settings-base-branch") != nil }
    try assertDetailHeader("releash", host)
    XCTAssertNotNil(host.element("/work"))
    try assertGroup(
      ["/work", "settings-base-branch"],
      excluding: ["settings-notion-token", "settings-remove-repository"], host)
    try assertGroup(
      ["settings-notion-token", "settings-notion-database", "settings-notion-validate"],
      excluding: ["settings-base-branch", "settings-remove-repository"], host)
    try assertGroup(
      ["settings-remove-repository"], excluding: ["settings-base-branch", "settings-notion-token"],
      host)
    try assertDescription("Base branch", "ファイルタブで「base と比較」を選んだときに、比べる相手になるブランチです。", host)
    // When
    try await eventuallyView(host) {
      host.element("settings-notion-database")?.value == "test-database"
        && host.element("settings-notion-validate")?.enabled == true
    }
    try await host.press("settings-notion-validate")
    // Then
    try await eventuallyView(host) { host.element("settings-notion-label-Status") != nil }
    for property in ["Name", "Status"] {
      let toggle = try XCTUnwrap(host.element("settings-notion-label-\(property)"))
      XCTAssertTrue(toggle.enabled)
      XCTAssertEqual(toggle.role, kAXCheckBoxRole)
    }
    try assertDescription("Labels", "新しい Worktree のダイアログで、Notion task を絞り込むのに使います。", host)
    // When
    try await back(host, title: "releash")
    // Then
    try await eventuallyView(host) {
      host.element("settings-base-branch") == nil
        && host.texts().contains(where: { $0.contains("/work") })
    }
    await model.stop()
  }

  @MainActor func test_プロバイダー設定_B010_B018_Providersのまとまりと説明と末尾の再確認() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.providerAvailability = testProviderAvailability() }
    let model = try await testModel(http)
    let host = await ViewHarness(SettingsView(model: model), size: NSSize(width: 1024, height: 800))
    defer { host.close() }
    // When
    try await host.press("Providers")
    // Then
    try await eventuallyView(host) { host.element("settings-provider-executable-claude") != nil }
    let visibleOrder = try descendants(window(host)).compactMap {
      attribute($0, kAXIdentifierAttribute) as? String
    }
    let refreshIndex = try XCTUnwrap(visibleOrder.firstIndex(of: "settings-provider-refresh"))
    for (provider, heading, other) in [
      ("claude", "Claude Code", "codex"), ("codex", "Codex", "claude"),
    ] {
      XCTAssertNotNil(host.element(heading))
      try assertGroup(
        ["settings-provider-executable-\(provider)", "settings-provider-reset-\(provider)"],
        excluding: ["settings-provider-refresh", "settings-provider-executable-\(other)"], host)
      XCTAssertLessThan(
        try XCTUnwrap(visibleOrder.firstIndex(of: "settings-provider-executable-\(provider)")),
        refreshIndex)
    }
    XCTAssertNotNil(host.element("可用性を再確認"))
    try assertDescription("有効な実行ファイル", "上書きがあればその値、無ければ既定の実行ファイルです。", host)
    try assertDescription("解決された実行ファイル", "有効な実行ファイルを PATH から探した結果です。", host)
    try assertDescription("実行ファイルの上書き", "既定と別の実行ファイルを使うときに指定します。", host)
    await model.stop()
  }

  @MainActor func test_ワークフロー設定_B011_B012_B013_B020_Workflowsの印と診断と詳細の往復() async throws {
    // Given
    let http = TestHTTPClient()
    testWorkflowSettings(http)
    http.state.withLock {
      $0.diagnostics.items.items[2].severity.value = .error
      $0.diagnostics.items.items[3].severity.value = .error
      $0.diagnostics.workflowSummaries.entries["01_author-spec"]?.infoCount = 0
      $0.diagnostics.workflowSummaries.entries["broken"] = .with { $0.errorCount = 1 }
      $0.diagnostics.facetSummaries.entries["policy/coding"] = .with { $0.errorCount = 1 }
      $0.diagnostics.total.errorCount = 3
      $0.diagnostics.total.infoCount = 1
    }
    let model = try await testModel(http)
    let host = await ViewHarness(SettingsView(model: model), size: NSSize(width: 1024, height: 800))
    defer { host.close() }
    // When
    try await host.press("Workflows")
    // Then
    try await eventuallyView(host) {
      host.element("settings-workflow-01_author-spec")?.label.contains("error: 1") == true
    }
    XCTAssertNotNil(host.element("定義"))
    XCTAssertNotNil(host.element("Facet"))
    XCTAssertNotNil(host.element("定義のフォルダを外部エディタで開く"))
    let running = try XCTUnwrap(host.element("settings-workflow-01_author-spec"))
    XCTAssertTrue(running.label.contains("01_author-spec"))
    XCTAssertTrue(running.label.contains("実行中"))
    XCTAssertTrue(running.label.contains("error: 1"))
    XCTAssertTrue(
      try XCTUnwrap(host.element("settings-workflow-builtin-flow")).label
        .components(separatedBy: ", ").contains("builtin"))
    XCTAssertNotNil(host.element("error: 3"))
    XCTAssertNotNil(host.element("info: 1"))
    let rowBounds = try ["01_author-spec", "builtin-flow", "broken"].map {
      try frame("settings-workflow-\($0)", host)
    }
    for total in ["error: 3", "info: 1"] {
      XCTAssertTrue(
        try elements(total, host).map(screenFrame).contains { summary in
          rowBounds.allSatisfy { !$0.contains(summary) }
        }, total)
    }
    for (name, builtin) in [("01_author-spec", false), ("builtin-flow", true)] {
      // When
      try await host.press("settings-workflow-\(name)")
      // Then
      try await eventuallyView(host) {
        host.element("settings-workflow-\(name)") == nil && host.element(name) != nil
      }
      try assertDetailHeader(name, host)
      if builtin {
        try await assertRemainsAbsent("settings-open-workflow-in-editor", host)
      } else {
        try await eventuallyView(host) { host.element("settings-open-workflow-in-editor") != nil }
        try assertTopRight("settings-open-workflow-in-editor", title: name, host)
      }
      // When
      try await back(host, title: name)
      // Then
      try await eventuallyView(host) { host.element("settings-workflows-mode") != nil }
    }
    // When
    try await host.press("Facet")
    for (key, builtin) in [("coding", false), ("builtin-facet", true)] {
      try await eventuallyView(host) { host.element("settings-facet-policy-\(key)") != nil }
      try await host.press("settings-facet-policy-\(key)")
      // Then
      try await eventuallyView(host) { host.element("settings-facet-kind") == nil }
      let heading = try detailHeading(key, host)
      let kinds = try elements(containing: "policy", host).map(screenFrame)
      XCTAssertTrue(kinds.contains { abs($0.midY - heading.midY) < max($0.height, heading.height) })
      try assertDetailHeader(key, host)
      if builtin {
        try await assertRemainsAbsent("settings-open-facet-in-editor", host)
      } else {
        try await eventuallyView(host) { host.element("settings-open-facet-in-editor") != nil }
        try assertTopRight("settings-open-facet-in-editor", title: key, host)
      }
      // When
      try await back(host, title: key)
      // Then
      try await eventuallyView(host) { host.element("settings-facet-policy-\(key)") != nil }
    }
    await model.stop()
  }

  @MainActor func test_システム設定_B014_B019_Systemの四つのまとまりと説明() async throws {
    // Given
    let model = try await testModel(TestHTTPClient())
    let host = await ViewHarness(SettingsView(model: model), size: NSSize(width: 1024, height: 800))
    defer { host.close() }
    // When
    try await host.press("System")
    // Then
    try await eventuallyView(host) { host.element("settings-auto-launch") != nil }
    let headings = ["更新", "ログイン項目", "CLI の設置", "Telemetry"]
    for heading in headings { XCTAssertNotNil(host.element(heading)) }
    for (label, description) in [
      ("nightly アップデートを受け入れる", "毎日作られる開発中の版も、更新の対象にします。"),
      ("最小化状態で起動", "ログイン時に起動したときは、ウィンドウを開きません。"),
      ("Performance telemetry", "サーバの動作の計測値（metrics と traces）を送ります。"),
      ("Crash reporting", "異常終了したときに、その記録を送ります。"),
    ] { try assertDescription(label, description, host) }
    await model.stop()
  }

  @MainActor func test_設定の永続化_B016_Terminal起動コマンドの確定値を開き直して表示する() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let host = await ViewHarness(SettingsView(model: model), size: NSSize(width: 1024, height: 800))
    try await eventuallyView(host) { host.element("settings-terminal-command")?.enabled == true }
    // When
    try await host.focusTextField()
    try await host.setTextField("echo accepted")
    let enter = try XCTUnwrap(
      NSEvent.keyEvent(
        with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
        windowNumber: host.window.windowNumber, context: nil, characters: "\r",
        charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36))
    host.window.sendEvent(enter)
    try await eventually { !http.requests("UpdateTerminalStartupCommand").isEmpty }
    let request = try Releash_Client_V1_UpdateTerminalStartupCommandRequest(
      serializedBytes: XCTUnwrap(http.requests("UpdateTerminalStartupCommand").last?.message))
    XCTAssertEqual(request.command, "echo accepted")
    http.state.withLock { $0.terminalStartupCommand = request.command }
    var payload = Releash_Client_V1_StatePayload()
    payload.terminalStartupCommand.value = request.command
    try http.publish(payload, target: "terminal-startup-command")
    try await eventuallyView(host) {
      host.element("settings-terminal-command")?.value == "echo accepted"
    }
    host.close()
    let reopened = await ViewHarness(
      SettingsView(model: model), size: NSSize(width: 1024, height: 800))
    defer { reopened.close() }
    // Then
    try await eventuallyView(reopened) {
      reopened.element("settings-terminal-command")?.value == "echo accepted"
    }
    await model.stop()
  }

  @MainActor private var screenHeight: CGFloat { NSScreen.screens.first?.frame.height ?? 0 }

  private func attribute(_ element: AXUIElement, _ key: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, key as CFString, &value) == .success else {
      return nil
    }
    return value
  }

  private func descendants(_ element: AXUIElement) -> [AXUIElement] {
    [element]
      + (attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []).flatMap(descendants)
  }

  @MainActor private func window(_ host: ViewHarness) throws -> AXUIElement {
    let app = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
    return try XCTUnwrap(
      (attribute(app, kAXWindowsAttribute) as? [AXUIElement])?.first {
        attribute($0, kAXTitleAttribute) as? String == host.window.title
      })
  }

  @MainActor private func elements(_ name: String, _ host: ViewHarness) throws -> [AXUIElement] {
    try descendants(window(host)).dropFirst().filter { item in
      [kAXIdentifierAttribute, kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute]
        .contains {
          attribute(item, $0) as? String == name
        }
    }
  }

  @MainActor private func elements(containing name: String, _ host: ViewHarness) throws
    -> [AXUIElement]
  {
    try descendants(window(host)).dropFirst().filter { item in
      [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute].contains {
        (attribute(item, $0) as? String)?.contains(name) == true
      }
    }
  }

  private func screenFrame(_ element: AXUIElement) throws -> CGRect {
    let raw = try XCTUnwrap(attribute(element, "AXFrame"))
    var rect = CGRect.zero
    XCTAssertTrue(AXValueGetValue(raw as! AXValue, .cgRect, &rect))
    return rect
  }

  @MainActor private func frame(_ name: String, _ host: ViewHarness) throws -> CGRect {
    let matches = try elements(name, host)
    let roles = ["AXHeading", kAXStaticTextRole]
    let match =
      roles.lazy.compactMap { role in
        matches.first { self.attribute($0, kAXRoleAttribute) as? String == role }
      }.first ?? matches.first
    return try screenFrame(XCTUnwrap(match, name))
  }

  @MainActor private func assertGroup(
    _ names: [String], excluding others: [String], _ host: ViewHarness
  ) throws {
    let members = try names.map { try XCTUnwrap(elements($0, host).first, $0) }
    let excluded = try others.flatMap { try elements($0, host) }
    XCTAssertTrue(
      try descendants(window(host)).contains { container in
        guard self.attribute(container, kAXRoleAttribute) as? String == kAXGroupRole else {
          return false
        }
        let contents = descendants(container)
        return members.allSatisfy { member in contents.contains { CFEqual($0, member) } }
          && excluded.allSatisfy { other in !contents.contains { CFEqual($0, other) } }
      }, names.joined(separator: ", "))
  }

  @MainActor private func assertRow(_ label: String, control: String, _ host: ViewHarness) throws {
    let labels = try elements(label, host).filter {
      attribute($0, kAXRoleAttribute) as? String == kAXStaticTextRole
    }
    let left = try screenFrame(XCTUnwrap(labels.first, label))
    let right = try frame(control, host)
    XCTAssertLessThanOrEqual(left.maxX, right.minX)
    XCTAssertLessThan(abs(left.midY - right.midY), max(left.height, right.height))
  }

  @MainActor private func assertDescription(_ label: String, _ text: String, _ host: ViewHarness)
    throws
  {
    let view = try XCTUnwrap(host.window.contentView)
    func scrollViews(_ view: NSView) -> [NSScrollView] {
      (view as? NSScrollView).map { [$0] } ?? view.subviews.flatMap(scrollViews)
    }
    for scroll in scrollViews(view) {
      guard let document = scroll.documentView else { continue }
      var target = try frame(text, host)
      target.origin.y = screenHeight - target.maxY
      let local = document.convert(host.window.convertFromScreen(target), from: nil)
      if document.bounds.contains(local) { document.scrollToVisible(local) }
    }
    view.layoutSubtreeIfNeeded()
    host.window.displayIfNeeded()
    let description = try frame(text, host)
    let labels = try elements(label, host).map(screenFrame)
    XCTAssertTrue(
      labels.contains { $0.maxY <= description.minY && abs($0.minX - description.minX) < 3 }, label)
    host.window.appearance = NSAppearance(named: .aqua)
    view.layoutSubtreeIfNeeded()
    host.window.displayIfNeeded()
    var screen = description
    screen.origin.y = screenHeight - description.maxY
    let region = view.convert(host.window.convertFromScreen(screen), from: nil).integral
    XCTAssertTrue(view.bounds.contains(region), text)
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: region))
    view.cacheDisplay(in: region, to: bitmap)
    let colors = (0..<bitmap.pixelsHigh).flatMap { y in
      (0..<bitmap.pixelsWide).compactMap { x in bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) }
    }
    let ink = colors.filter {
      $0.redComponent < 0.7 && $0.greenComponent < 0.7 && $0.blueComponent < 0.7
    }
    XCTAssertGreaterThan(ink.count, 5, text)
    XCTAssertTrue(
      ink.allSatisfy {
        abs($0.redComponent - $0.greenComponent) < 0.08
          && abs($0.greenComponent - $0.blueComponent) < 0.08 && $0.redComponent > 0.15
      }, text)
  }

  private func texts(in element: AXUIElement) -> [String] {
    descendants(element).flatMap { item in
      [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute].compactMap {
        attribute(item, $0) as? String
      }
    }
  }

  @MainActor private func repositoryRow(_ name: String, path: String, _ host: ViewHarness) throws
    -> AXUIElement
  {
    let rows = try descendants(window(host)).filter {
      [kAXRowRole, kAXButtonRole].contains(
        attribute($0, kAXRoleAttribute) as? String ?? "")
    }
    return try XCTUnwrap(
      rows.first { row in
        let text = texts(in: row)
        return text.contains { $0.contains(name) } && text.contains { $0.contains(path) }
      }, "B-004: 同じ Repository の行に \(name) と \(path) が出る")
  }

  @MainActor private func assertDetailHeader(_ name: String, _ host: ViewHarness) throws {
    let heading = try detailHeading(name, host)
    let bounds = try screenFrame(window(host))
    XCTAssertGreaterThanOrEqual(heading.minY, bounds.minY)
    XCTAssertLessThan(heading.maxY, bounds.minY + bounds.height / 4, "詳細ページの上部に \(name) が出る")
  }

  @MainActor private func detailHeading(_ name: String, _ host: ViewHarness) throws -> CGRect {
    let candidates = try elements(containing: name, host).filter {
      ["AXHeading", kAXStaticTextRole].contains(attribute($0, kAXRoleAttribute) as? String ?? "")
    }
    return try XCTUnwrap(candidates.map(screenFrame).min { $0.minY < $1.minY }, name)
  }

  @MainActor private func assertRemainsAbsent(
    _ identifier: String, _ host: ViewHarness, file: StaticString = #filePath, line: UInt = #line
  ) async throws {
    let deadline = ContinuousClock.now + .seconds(5)
    repeat {
      await host.refresh()
      guard host.element(identifier) == nil else {
        XCTFail("B-020: \(identifier) が表示された: \(host.debugDump())", file: file, line: line)
        return
      }
      try await Task.sleep(for: .milliseconds(20))
    } while ContinuousClock.now < deadline
    await host.refresh()
    XCTAssertNil(host.element(identifier), file: file, line: line)
  }

  @MainActor private func assertTopRight(_ control: String, title: String, _ host: ViewHarness)
    throws
  {
    let heading = try detailHeading(title, host)
    let button = try frame(control, host)
    XCTAssertGreaterThan(button.minX, heading.maxX)
    XCTAssertLessThan(abs(button.midY - heading.midY), max(button.height, heading.height))
  }

  @MainActor private func selectRepository(_ name: String, _ host: ViewHarness) async throws {
    let candidates = try elements(containing: name, host)
    if let button = candidates.first(where: { item in
      var actions: CFArray?
      AXUIElementCopyActionNames(item, &actions)
      return (actions as? [String] ?? []).contains(kAXPressAction)
    }) {
      XCTAssertEqual(AXUIElementPerformAction(button, kAXPressAction as CFString), .success)
    } else {
      try await host.selectRow(name)
    }
  }

  @MainActor private func back(_ host: ViewHarness, title: String) async throws {
    let heading = try detailHeading(title, host)
    let buttons = try descendants(window(host)).dropFirst().filter { item in
      guard attribute(item, kAXRoleAttribute) as? String == kAXButtonRole,
        let rect = try? screenFrame(item)
      else { return false }
      return rect.maxX <= heading.minX
        && abs(rect.midY - heading.midY) < max(rect.height, heading.height)
    }
    XCTAssertEqual(buttons.count, 1, "詳細の左上に戻るボタンがある")
    XCTAssertEqual(
      AXUIElementPerformAction(try XCTUnwrap(buttons.first), kAXPressAction as CFString), .success)
    await host.refresh()
  }
  @MainActor private func localFrame(_ name: String, _ host: ViewHarness) throws -> NSRect {
    var screen = try frame(name, host)
    screen.origin.y = screenHeight - screen.maxY
    return try XCTUnwrap(host.window.contentView).convert(
      host.window.convertFromScreen(screen), from: nil)
  }

  @MainActor private func assertIconPresent(_ title: String, _ host: ViewHarness) throws {
    let view = try XCTUnwrap(host.window.contentView)
    host.window.appearance = NSAppearance(named: .aqua)
    view.layoutSubtreeIfNeeded()
    host.window.displayIfNeeded()
    let region = try localFrame(title, host).integral.intersection(view.bounds)
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: region))
    view.cacheDisplay(in: region, to: bitmap)
    let actual = try IconRaster(bitmap: bitmap)
    let scale = CGFloat(bitmap.pixelsWide) / region.width
    for svg in LucideAcceptanceReference.svg.values {
      let image = try XCTUnwrap(NSImage(data: Data(svg.utf8)))
      for size in 10...20 {
        let reference = try IconRaster(image: image, size: size, scale: scale)
        if actual.find(reference, color: nil) != nil { return }
      }
    }
    XCTFail("B-001: \(title) にアイコンが描画されている")
  }
}

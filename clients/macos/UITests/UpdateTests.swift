import AppKit
import ApplicationServices
import XCTest

@MainActor final class UpdateTests: XCTestCase, @unchecked Sendable {
  private var fixture: Fixture!
  private var update: UpdateFixture!
  private var app: XCUIApplication!

  @MainActor override func setUp() async throws {
    executionTimeAllowance = 300
    fixture = try Fixture()
    update = try UpdateFixture(fixture: fixture)
  }

  nonisolated override func tearDownWithError() throws {
    try MainActor.assumeIsolated {
      app?.terminate()
      update?.cleanUp()
      try fixture?.cleanUp()
    }
  }

  @MainActor func test_更新適用_終了時に差し替えてサーバとworkflowを継続する() async throws {
    try await applyUpdate(immediately: false)
  }

  @MainActor func test_更新適用_今すぐ差し替えて再起動しサーバとworkflowを継続する() async throws {
    try await applyUpdate(immediately: true)
  }

  @MainActor func test_更新設定_初期表示とサーバ未接続での変更を再起動後も保持する() async throws {
    // Given
    try await update.prepare()
    try """
    <?xml version="1.0"?><rss version="2.0"><channel><title>Update test</title></channel></rss>
    """.write(
      to: fixture.directory.appendingPathComponent("appcast.xml"), atomically: true, encoding: .utf8
    )
    let unavailable = fixture.directory.appendingPathComponent("unavailable-data")
    try Data().write(to: unavailable)
    app = XCUIApplication(url: update.application)
    app.launchEnvironment = fixture.environment
    app.launchEnvironment["RELEASH_DATA_DIR"] = unavailable.path
    app.launch()
    XCTAssertTrue(app.staticTexts["サーバに接続できません"].waitForExistence(timeout: 30))
    // When
    openSystemSettings()
    let automatic = app.switches["自動更新"]
    let nightly = app.switches["nightly アップデートを受け入れる"]
    // Then
    XCTAssertTrue(automatic.waitForExistence(timeout: 10), app.debugDescription)
    XCTAssertEqual((automatic.value as? NSNumber)?.boolValue, true)
    XCTAssertEqual((nightly.value as? NSNumber)?.boolValue, false)
    XCTAssertTrue(automatic.isEnabled)
    XCTAssertTrue(nightly.isEnabled)
    // When
    // The accessibility frame includes the label; the switch is at the trailing edge.
    automatic.coordinate(withNormalizedOffset: CGVector(dx: 0.97, dy: 0.5)).click()
    try await waitUntil { (automatic.value as? NSNumber)?.boolValue == false }
    nightly.coordinate(withNormalizedOffset: CGVector(dx: 0.97, dy: 0.5)).click()
    try await waitUntil { (nightly.value as? NSNumber)?.boolValue == true }
    app.typeKey("q", modifierFlags: .command)
    XCTAssertTrue(app.wait(for: .notRunning, timeout: 15))
    app.launch()
    XCTAssertTrue(app.staticTexts["サーバに接続できません"].waitForExistence(timeout: 30))
    openSystemSettings()
    // Then
    XCTAssertTrue(automatic.waitForExistence(timeout: 10), app.debugDescription)
    XCTAssertEqual((automatic.value as? NSNumber)?.boolValue, false)
    XCTAssertEqual((nightly.value as? NSNumber)?.boolValue, true)
  }

  @MainActor func test_Debugアプリ_appcastを読まず更新を案内しない() async throws {
    // Given
    guard
      NSRunningApplication.runningApplications(withBundleIdentifier: "com.releash.macos.dev")
        .isEmpty
    else {
      XCTFail("B-019 を検証できません。起動中の Debug アプリを閉じてから実行してください")
      return
    }
    try await update.prepare(debug: true)
    let unavailable = fixture.directory.appendingPathComponent("unavailable-data")
    try Data().write(to: unavailable)
    app = XCUIApplication(url: update.application)
    app.launchEnvironment = fixture.environment
    app.launchEnvironment["RELEASH_DATA_DIR"] = unavailable.path
    // When
    app.launch()
    XCTAssertTrue(app.staticTexts["サーバに接続できません"].waitForExistence(timeout: 30))
    // Then
    let deadline = Date().addingTimeInterval(15)
    while Date() < deadline {
      XCTAssertFalse(
        FileManager.default.fileExists(
          atPath: fixture.directory.appendingPathComponent("feed-requested").path))
      XCTAssertFalse(app.buttons["更新"].exists)
      XCTAssertFalse(app.windows["SUUpdateAlert"].exists)
      try await Task.sleep(for: .milliseconds(100))
    }
  }

  @MainActor private func openSystemSettings() {
    app.typeKey(",", modifierFlags: .command)
    let system = app.buttons["System"]
    XCTAssertTrue(system.waitForExistence(timeout: 10), app.debugDescription)
    system.click()
  }

  @MainActor func test_手動更新確認_メニューからSparkleの標準画面を開く() async throws {
    // Given
    try Data().write(to: fixture.directory.appendingPathComponent("hold-feed"))
    try await update.prepare(automatic: false)
    try await fixture.startServer()
    app = XCUIApplication(url: update.application)
    app.launchEnvironment = fixture.environment
    app.launchArguments = ["-AppleLanguages", "(en)"]
    app.launch()
    XCTAssertTrue(app.buttons["Worktreeを作成"].waitForExistence(timeout: 30))
    // When
    let status = app.descendants(matching: .statusItem)["Releash"]
    let running = try XCTUnwrap(
      NSRunningApplication.runningApplications(withBundleIdentifier: update.identifier).first)
    let application = AXUIElementCreateApplication(running.processIdentifier)
    var menuBar: CFTypeRef?
    XCTAssertEqual(
      AXUIElementCopyAttributeValue(application, kAXExtrasMenuBarAttribute as CFString, &menuBar),
      .success)
    let bar = try XCTUnwrap(menuBar)
    var children: CFTypeRef?
    XCTAssertEqual(
      AXUIElementCopyAttributeValue(
        bar as! AXUIElement, kAXChildrenAttribute as CFString, &children),
      .success)
    let items = try XCTUnwrap(children as? [AXUIElement])
    let tooltips = items.compactMap { item -> String? in
      var help: CFTypeRef?
      guard AXUIElementCopyAttributeValue(item, kAXHelpAttribute as CFString, &help) == .success
      else { return nil }
      return help as? String
    }
    // Then
    XCTAssertEqual(tooltips.count, 1)
    XCTAssertEqual(tooltips.first?.components(separatedBy: ":").first, "Releash")
    XCTAssertFalse(tooltips.contains { $0.contains("Releash Preview") })
    XCTAssertFalse(app.debugDescription.contains("Releash Preview"))
    // When
    status.click()
    // Then
    XCTAssertTrue(app.menuItems["Releash について"].exists)
    XCTAssertTrue(app.menuItems["Releash を終了"].exists)
    XCTAssertFalse(app.debugDescription.contains("Releash Preview"))
    // When
    app.menuItems["Releash について"].click()
    // Then
    XCTAssertTrue(app.staticTexts["Releash"].waitForExistence(timeout: 10), app.debugDescription)
    XCTAssertFalse(app.debugDescription.contains("Releash Preview"))
    app.typeKey("w", modifierFlags: .command)
    // When
    status.click()
    let check = app.menuItems["アップデートを確認"]
    XCTAssertTrue(check.waitForExistence(timeout: 10), app.debugDescription)
    XCTAssertTrue(check.isEnabled)
    check.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).click()
    // Then
    try await waitUntil {
      FileManager.default.fileExists(
        atPath: self.fixture.directory.appendingPathComponent("feed-requested").path)
    }
    app.descendants(matching: .statusItem)["Releash"].click()
    XCTAssertTrue(check.waitForExistence(timeout: 10))
    XCTAssertTrue(check.isEnabled)
    check.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).click()
    try FileManager.default.removeItem(at: fixture.directory.appendingPathComponent("hold-feed"))
    XCTAssertTrue(
      app.windows["SUUpdateAlert"].buttons["SPUUserUpdateChoiceInstall"].waitForExistence(
        timeout: 15), app.debugDescription)
    XCTAssertTrue(
      app.staticTexts.matching(
        NSPredicate(
          format: "value CONTAINS %@ OR label CONTAINS %@", update.updatedVersion,
          update.updatedVersion)
      ).firstMatch
        .exists)
    XCTAssertEqual(app.webViews.count, 0)
    // Given
    app.descendants(matching: .statusItem)["Releash"].click()
    app.menuItems["開く"].click()
    // When
    app.descendants(matching: .statusItem)["Releash"].click()
    XCTAssertTrue(check.waitForExistence(timeout: 10))
    XCTAssertTrue(check.isEnabled)
    check.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).click()
    // Then
    try await waitUntil {
      guard
        NSWorkspace.shared.frontmostApplication?.processIdentifier == running.processIdentifier
      else { return false }
      var window: CFTypeRef?
      guard
        AXUIElementCopyAttributeValue(application, kAXFocusedWindowAttribute as CFString, &window)
          == .success,
        let window
      else { return false }
      var identifier: CFTypeRef?
      return AXUIElementCopyAttributeValue(
        window as! AXUIElement, kAXIdentifierAttribute as CFString, &identifier) == .success
        && identifier as? String == "SUUpdateAlert"
    }
  }

  @MainActor private func applyUpdate(immediately: Bool) async throws {
    // Given
    try Data().write(to: fixture.directory.appendingPathComponent("hold-feed"))
    try await update.prepare()
    try await fixture.startServer()
    let repository = try await fixture.repository("update-repository")
    let (client, headers) = try fixture.client()
    var definition = Releash_Client_V1_SaveWorkflowSourceRequest()
    definition.source = """
      name: update-continuity
      description: Update continuity regression
      nodes:
        main:
          command: sleep 3600
      """
    let saved = try await client.saveWorkflowSource(request: definition, headers: headers).result
      .get()
    guard case .success = saved.variant else {
      XCTFail("Workflow could not be saved: \(saved)")
      return
    }
    var request = Releash_Client_V1_StartWorkflowRequest()
    request.workflowName = "update-continuity"
    request.worktreePath = repository.path
    let execution = try await client.startWorkflow(request: request, headers: headers).result.get()
    try await fixture.waitForState(target: "workspaces") { payload in
      guard case .workspaces(let workspaces) = payload.value else { return false }
      return Fixture.hasRunningWorkflow(
        workspaces, repository: repository.path, executionID: execution.value)
    }
    let server = try await client.getServerInfo(request: .init(), headers: headers).result.get()
    XCTAssertEqual(server.release, update.currentVersion)
    app = XCUIApplication(url: update.application)
    app.launchEnvironment = fixture.environment
    app.launchArguments = ["-AppleLanguages", "(en)"]
    app.launch()
    XCTAssertTrue(app.buttons["Worktreeを作成"].waitForExistence(timeout: 30), app.debugDescription)
    let originalProcess = try XCTUnwrap(
      NSRunningApplication.runningApplications(withBundleIdentifier: update.identifier).first
    ).processIdentifier
    // When: Sparkle downloads automatically, without pressing a check or download button.
    try await waitUntil {
      FileManager.default.fileExists(
        atPath: self.fixture.directory.appendingPathComponent("feed-requested").path)
    }
    // Then
    app.descendants(matching: .statusItem)["Releash"].click()
    let check = app.menuItems["アップデートを確認"]
    XCTAssertTrue(check.waitForExistence(timeout: 10))
    XCTAssertFalse(check.isEnabled)
    app.typeKey(.escape, modifierFlags: [])
    // When
    try FileManager.default.removeItem(at: fixture.directory.appendingPathComponent("hold-feed"))
    try await waitUntil {
      FileManager.default.fileExists(
        atPath: self.fixture.directory.appendingPathComponent("download-requested").path)
    }
    // Then
    XCTAssertEqual(try update.installedVersion(), update.currentVersion)
    XCTAssertTrue(app.buttons["更新"].waitForExistence(timeout: 10))
    XCTAssertTrue(app.staticTexts[update.updatedVersion].exists)
    XCTAssertFalse(app.buttons["更新"].isEnabled)
    app.descendants(matching: .statusItem)["Releash"].click()
    XCTAssertTrue(check.waitForExistence(timeout: 10))
    XCTAssertFalse(check.isEnabled)
    app.typeKey(.escape, modifierFlags: [])
    XCTAssertTrue(app.state == .runningForeground || app.state == .runningBackground)
    try Data().write(to: fixture.directory.appendingPathComponent("release-download"))
    try await waitUntil { self.app.buttons["更新"].isEnabled }
    XCTAssertEqual(try update.installedVersion(), update.currentVersion)
    XCTAssertEqual(
      NSRunningApplication.runningApplications(withBundleIdentifier: update.identifier)
        .map(\.processIdentifier), [originalProcess])
    // When
    if immediately {
      app.buttons["更新"].click()
      let install = app.windows["SUUpdateAlert"].buttons["SPUUserUpdateChoiceInstall"]
      XCTAssertTrue(install.waitForExistence(timeout: 15), app.debugDescription)
      XCTAssertTrue(
        app.windows["SUUpdateAlert"].staticTexts.matching(
          NSPredicate(
            format: "value CONTAINS %@ OR label CONTAINS %@", update.updatedVersion,
            update.updatedVersion)
        ).firstMatch.exists)
      XCTAssertEqual(app.webViews.count, 0, "Release notes must not be displayed")
      install.click()
    } else {
      app.typeKey("q", modifierFlags: .command)
    }
    try await waitUntil { try self.update.installedVersion() == self.update.updatedVersion }
    // Then
    if !immediately {
      XCTAssertTrue(app.wait(for: .notRunning, timeout: 15))
      app.launch()
    }
    try await waitUntil {
      NSRunningApplication.runningApplications(withBundleIdentifier: self.update.identifier)
        .contains { $0.processIdentifier != originalProcess }
    }
    XCTAssertTrue(app.buttons["Worktreeを作成"].waitForExistence(timeout: 30), app.debugDescription)
    let connection = app.menuButtons["footer-server"]
    XCTAssertTrue(connection.waitForExistence(timeout: 30), app.debugDescription)
    XCTAssertTrue(connection.label.contains(update.currentVersion), connection.label)
    XCTAssertTrue(app.staticTexts["再起動で更新できます"].waitForExistence(timeout: 10))
    let current = try await client.getServerInfo(request: .init(), headers: headers).result.get()
    XCTAssertEqual(current.pid, server.pid)
    XCTAssertEqual(current.processStartedAt, server.processStartedAt)
    XCTAssertEqual(current.daemonID, server.daemonID)
    try await fixture.waitForState(target: "workspaces") { payload in
      guard case .workspaces(let workspaces) = payload.value else { return false }
      return Fixture.hasRunningWorkflow(
        workspaces, repository: repository.path, executionID: execution.value)
    }
    try await waitUntil {
      self.app.buttons["worktree:main"].firstMatch.label.contains("update-continuity")
    }
  }

  @MainActor private func waitUntil(_ condition: () throws -> Bool) async throws {
    let deadline = Date().addingTimeInterval(45)
    while Date() < deadline {
      if try condition() { return }
      try await Task.sleep(for: .milliseconds(100))
    }
    XCTFail(app?.debugDescription ?? "Update condition timed out")
    throw NSError(domain: "UpdateTests", code: 1)
  }
}

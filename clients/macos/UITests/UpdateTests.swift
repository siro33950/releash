import AppKit
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
    app.descendants(matching: .statusItem)["Releash"].click()
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
    XCTAssertFalse(check.isEnabled)
    app.typeKey(.escape, modifierFlags: [])
    try FileManager.default.removeItem(at: fixture.directory.appendingPathComponent("hold-feed"))
    XCTAssertTrue(
      app.windows["SUUpdateAlert"].buttons["SPUUserUpdateChoiceInstall"].waitForExistence(
        timeout: 15), app.debugDescription)
    XCTAssertTrue(
      app.staticTexts.matching(
        NSPredicate(format: "value CONTAINS %@ OR label CONTAINS %@", "0.4.21", "0.4.21")
      ).firstMatch
        .exists)
    XCTAssertEqual(app.webViews.count, 0)
  }

  @MainActor private func applyUpdate(immediately: Bool) async throws {
    // Given
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
    let server = try await client.getServerInfo(request: .init(), headers: headers).result.get()
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
        atPath: self.fixture.directory.appendingPathComponent("download-requested").path)
    }
    // Then
    XCTAssertEqual(try update.installedVersion(), "0.4.20")
    XCTAssertTrue(app.buttons["更新"].waitForExistence(timeout: 10))
    XCTAssertFalse(app.buttons["更新"].isEnabled)
    XCTAssertTrue(app.state == .runningForeground || app.state == .runningBackground)
    try Data().write(to: fixture.directory.appendingPathComponent("release-download"))
    try await waitUntil { self.app.buttons["更新"].isEnabled }
    // When
    if immediately {
      app.buttons["更新"].click()
      let install = app.windows["SUUpdateAlert"].buttons["SPUUserUpdateChoiceInstall"]
      XCTAssertTrue(install.waitForExistence(timeout: 15), app.debugDescription)
      XCTAssertEqual(app.webViews.count, 0, "Release notes must not be displayed")
      install.click()
    } else {
      app.typeKey("q", modifierFlags: .command)
    }
    try await waitUntil { try self.update.installedVersion() == "0.4.21" }
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
    XCTAssertTrue(connection.label.contains("serving"), connection.label)
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

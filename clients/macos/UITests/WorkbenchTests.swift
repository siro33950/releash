import XCTest

final class WorkbenchTests: XCTestCase {
  var fixture: Fixture!
  var app: XCUIApplication!

  @MainActor override func setUp() async throws {
    fixture = try Fixture()
    app = XCUIApplication()
    app.launchEnvironment = fixture.environment
  }

  @MainActor override func tearDown() async throws {
    app?.terminate()
    try fixture?.cleanUp()
    app = nil
    fixture = nil
  }

  @MainActor func launch() {
    app.launch()
    XCTAssertTrue(app.buttons["Worktreeを作成"].waitForExistence(timeout: 30), app.debugDescription)
  }

  @MainActor func testServerStartsAndConnects() throws {
    XCTAssertFalse(try fixture.running())
    launch()
    XCTAssertTrue(try fixture.running())
  }

  @MainActor func testRepositoriesContainWorktreeCards() async throws {
    try await fixture.startServer()
    let (client, headers) = try fixture.client()
    var definition = Releash_Client_V1_SaveWorkflowSourceRequest()
    definition.source = """
      name: e2e-running
      description: E2E fixture
      nodes:
        main:
          command: sleep 3600
      """
    let saved = try await client.saveWorkflowSource(request: definition, headers: headers).result.get()
    guard case .success = saved.variant else {
      throw NSError(domain: "E2E", code: 3, userInfo: [NSLocalizedDescriptionKey: String(describing: saved.variant)])
    }
    _ = try await fixture.repository("first-repository", worktrees: ["feature-one"],
      workflows: ["e2e-running"], threads: ["worktree card context"])
    _ = try await fixture.repository("second-repository", worktrees: ["feature-two"])

    let first = fixture.directory.appendingPathComponent("first-repository")
    let second = fixture.directory.appendingPathComponent("second-repository")
    launch()
    let firstHeader = app.descendants(matching: .any).matching(NSPredicate(format: "identifier == %@", "repository:\(first.path)")).element
    let secondHeader = app.descendants(matching: .any).matching(NSPredicate(format: "identifier == %@", "repository:\(second.path)")).element
    let firstCard = app.buttons["worktree:feature-one"]
    let secondCard = app.buttons["worktree:feature-two"]
    for element in [firstHeader, secondHeader, firstCard, secondCard] {
      XCTAssertTrue(element.waitForExistence(timeout: 30), app.debugDescription)
    }
    XCTAssertTrue(firstHeader.label.hasPrefix("first-repository"))
    XCTAssertTrue(secondHeader.label.hasPrefix("second-repository"))
    XCTAssertEqual(firstCard.label, "feature-one")
    XCTAssertEqual(secondCard.label, "feature-two")
    XCTAssertGreaterThan(firstCard.frame.minY, firstHeader.frame.minY)
    XCTAssertLessThan(firstCard.frame.maxY, secondHeader.frame.minY)
    XCTAssertGreaterThan(secondCard.frame.minY, secondHeader.frame.minY)
    let execution = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label CONTAINS %@", "e2e-running"),
      object: app.buttons["worktree:main"].firstMatch)
    XCTAssertEqual(XCTWaiter.wait(for: [execution], timeout: 30), .completed, app.debugDescription)
  }

  @MainActor func testSplitAndMoveTab() async throws {
    try await fixture.startServer()
    _ = try await fixture.repository("pane-repository")
    launch()
    app.buttons.matching(identifier: "worktree:main").firstMatch.click()
    let open = app.menuButtons["開く"].firstMatch
    XCTAssertTrue(open.waitForExistence(timeout: 30))
    open.click()
    app.menuItems["Terminal"].click()
    open.click()
    app.menuItems["ファイル"].click()
    let tab = app.buttons["Terminal"].firstMatch
    XCTAssertTrue(tab.waitForExistence(timeout: 10))
    let edge = open.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 5))
    tab.click()
    tab.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
      .press(forDuration: 0.5, thenDragTo: edge)
    XCTAssertTrue(app.menuButtons.matching(identifier: "開く").element(boundBy: 1).waitForExistence(timeout: 10), app.debugDescription)
    let moved = app.scrollViews.buttons["Terminal"].firstMatch
    let retained = app.scrollViews.buttons["ファイル"].firstMatch
    let destination = app.scrollViews.containing(.button, identifier: "Terminal")
      .containing(.button, identifier: "ファイル").firstMatch
    XCTAssertFalse(destination.exists)
    let source = app.scrollViews.containing(.button, identifier: "Terminal")
      .menuButtons["開く"].firstMatch
    source.click()
    app.menuItems["Workflow 管理"].click()
    moved.click()
    moved.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
      .press(forDuration: 0.5, thenDragTo: retained.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)))
    XCTAssertTrue(destination.waitForExistence(timeout: 10), app.debugDescription)
    XCTAssertEqual(app.menuButtons.matching(identifier: "開く").count, 2)

  }

  @MainActor func testCreateWorktreeAndStartSession() async throws {
    try await fixture.startServer()
    _ = try await fixture.repository("creation-repository")
    let repository = fixture.directory.appendingPathComponent("creation-repository")
    launch()
    app.buttons["Worktreeを作成"].click()
    let picker = app.descendants(matching: .any)["repository-picker"].firstMatch
    XCTAssertTrue(picker.waitForExistence(timeout: 10), app.debugDescription)
    picker.click()
    app.menuItems[repository.path].click()
    app.disclosureTriangles["Advanced"].coordinate(withNormalizedOffset: CGVector(dx: 0.25, dy: 0.5)).click()
    let branch = app.descendants(matching: .any)["new-branch-field"].firstMatch
    XCTAssertTrue(branch.waitForExistence(timeout: 10), app.debugDescription)
    branch.click()
    branch.typeText("created-branch")
    app.sheets.firstMatch.scrollViews.firstMatch.scroll(byDeltaX: 0, deltaY: -250)
    app.radioButtons["Session"].click()
    let provider = app.descendants(matching: .any)["provider-picker"].firstMatch
    XCTAssertTrue(provider.waitForExistence(timeout: 10), app.debugDescription)
    provider.click()
    app.menuItems["Claude"].click()
    app.buttons["作成"].click()
    let card = app.buttons["worktree:created-branch"]
    XCTAssertTrue(card.waitForExistence(timeout: 30), app.debugDescription)
    let started = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label CONTAINS %@", "Claude"), object: card)
    XCTAssertEqual(XCTWaiter.wait(for: [started], timeout: 30), .completed, app.debugDescription)
    XCTAssertTrue(card.label.contains("Session,"), card.label)
    try await fixture.waitForLiveSession(repository: repository, branch: "created-branch", provider: .claude)
  }
}

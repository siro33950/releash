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

  @MainActor func testFooterShowsProviderUsageAndServerState() async throws {
    try fixture.cleanUp()
    fixture = try Fixture(providerUsage: true)
    app.launchEnvironment = fixture.environment
    try await fixture.startServer()
    let repository = try await fixture.repository("footer-repository")
    let (client, headers) = try fixture.client()
    var request = Releash_Client_V1_CreateAgentSessionRequest()
    request.provider = "claude"
    request.worktreePath = repository.path
    request.workspaceIdentity = repository.path
    request.rows = 24
    request.cols = 80
    request.callerRequestID = UUID().uuidString
    _ = try await client.createAgentSession(request: request, headers: headers).result.get()
    launch()
    let provider = app.buttons["footer-provider:claude"]
    XCTAssertTrue(provider.waitForExistence(timeout: 30), app.debugDescription)
    XCTAssertTrue(provider.label.contains("81% 5h"), provider.label)
    XCTAssertTrue(provider.label.contains("41% wk"), provider.label)
    XCTAssertTrue(provider.label.contains("使用率の警告"), provider.label)
    let server = app.menuButtons["footer-server"]
    XCTAssertTrue(server.waitForExistence(timeout: 30), app.debugDescription)
    XCTAssertTrue(server.label.contains("serving"), server.label)
    XCTAssertTrue(server.label.contains("compatible"), server.label)
    XCTAssertTrue(server.label.contains("v"), server.label)
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

  @MainActor func testFilesPanelOpensFileTabAndShowsDiff() async throws {
    try await fixture.startServer()
    let repository = try await fixture.repository("file-repository")
    let path = repository.appendingPathComponent("file.swift")
    try "let before = 1\n".write(to: path, atomically: true, encoding: .utf8)
    let git = URL(fileURLWithPath: "/usr/bin/git")
    try fixture.process(git, ["add", "file.swift"], at: repository)
    try fixture.process(git, ["-c", "user.name=E2E", "-c", "user.email=e2e@example.invalid", "-c", "commit.gpgsign=false", "commit", "-m", "file"], at: repository)
    try "let after = 2\n".write(to: path, atomically: true, encoding: .utf8)
    launch()
    app.buttons["worktree:main"].firstMatch.click()
    app.checkBoxes["ファイル"].click()
    let file = app.descendants(matching: .any).matching(identifier: "file:file.swift").firstMatch
    XCTAssertTrue(file.waitForExistence(timeout: 30), app.debugDescription)
    file.click()
    XCTAssertTrue(app.buttons["file.swift"].firstMatch.waitForExistence(timeout: 10), app.debugDescription)
    let after = app.webViews.staticTexts.matching(NSPredicate(format: "value CONTAINS %@", "after")).firstMatch
    let before = app.webViews.staticTexts.matching(NSPredicate(format: "value CONTAINS %@", "before")).firstMatch
    XCTAssertTrue(after.waitForExistence(timeout: 30), app.debugDescription)
    XCTAssertTrue(before.waitForExistence(timeout: 10), app.debugDescription)
    let mode = app.descendants(matching: .any).matching(identifier: "file-mode").firstMatch
    mode.click()
    XCTAssertTrue(after.waitForExistence(timeout: 10), app.debugDescription)
    let removed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: before)
    XCTAssertEqual(XCTWaiter.wait(for: [removed], timeout: 10), .completed, app.debugDescription)
    mode.click()
    XCTAssertTrue(before.waitForExistence(timeout: 10), app.debugDescription)

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
    app.menuItems["Terminal"].click()
    let tab = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Terminal ")).firstMatch
    XCTAssertTrue(tab.waitForExistence(timeout: 10))
    let terminalTitle = tab.label
    let edge = open.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 5))
    tab.click()
    tab.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
      .press(forDuration: 0.5, thenDragTo: edge)
    XCTAssertTrue(app.menuButtons.matching(identifier: "開く").element(boundBy: 1).waitForExistence(timeout: 10), app.debugDescription)
    let moved = app.scrollViews.buttons[terminalTitle].firstMatch
    let retained = app.scrollViews.buttons["Terminal 2"].firstMatch
    let destination = app.scrollViews.containing(.button, identifier: terminalTitle)
      .containing(.button, identifier: "Terminal 2").firstMatch
    XCTAssertFalse(destination.exists)
    let source = app.scrollViews.containing(.button, identifier: terminalTitle)
      .menuButtons["開く"].firstMatch
    source.click()
    app.menuItems["Terminal"].click()
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
    provider.menuItems["Claude"].click()
    app.buttons["作成"].click()
    let card = app.buttons["worktree:created-branch"]
    XCTAssertTrue(card.waitForExistence(timeout: 30), app.debugDescription)
    let started = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label CONTAINS %@", "Claude"), object: card)
    XCTAssertEqual(XCTWaiter.wait(for: [started], timeout: 30), .completed, app.debugDescription)
    XCTAssertTrue(card.label.contains("Session,"), card.label)
    try await fixture.waitForLiveSession(repository: repository, branch: "created-branch", provider: .claude)
  }
}


extension WorkbenchTests {
  @MainActor func testTerminalShowsCommandOutput() async throws {
    try await fixture.startServer()
    _ = try await fixture.repository("terminal-repository")
    launch()
    let worktree = app.buttons.matching(identifier: "worktree:main").firstMatch
    guard worktree.waitForExistence(timeout: 30) else { XCTFail("Worktreeが表示されません"); return }
    worktree.click()
    let open = app.menuButtons["開く"].firstMatch
    let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == true AND enabled == true"), object: open)
    guard await XCTWaiter.fulfillment(of: [ready], timeout: 30) == .completed else { XCTFail("開くメニューが有効になりません"); return }
    open.click()
    let item = app.menuItems["Terminal"]
    guard item.waitForExistence(timeout: 10), item.isHittable else { XCTFail("Terminalメニューを選択できません"); return }
    item.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).click()
    let tab = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Terminal ")).firstMatch
    guard tab.waitForExistence(timeout: 30) else { XCTFail("Terminalタブが作られません"); return }
    tab.click()
    let terminals = app.textViews.matching(NSPredicate(format: "identifier BEGINSWITH %@", "terminal:"))
    let terminal = terminals.firstMatch
    guard terminal.waitForExistence(timeout: 30), terminals.count == 1 else { XCTFail("作成したタブの端末ビューを取得できません"); return }
    terminal.click()
    terminal.typeText("printf 'releash-%s\\n' terminal-output\n")
    let output = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value CONTAINS %@", "releash-terminal-output"), object: terminal)
    XCTAssertEqual(XCTWaiter.wait(for: [output], timeout: 30), .completed, app.debugDescription)
  }
}

extension WorkbenchTests {
  @MainActor func testSessionTabShowsProviderTUIAndAcceptsInput() async throws {
    try await fixture.startServer()
    let repository = try await fixture.repository("session-tab-repository")
    var path = ""
    try await fixture.waitForState(target: "workspaces") { payload in
      guard case .workspaces(let value) = payload.value,
        let repo = value.repositories.items.first(where: { $0.path == repository.path }),
        let main = repo.branches.items.first(where: { $0.isMainWorktree }) else { return false }
      path = main.worktreePath
      return true
    }
    let (client, headers) = try fixture.client()
    var request = Releash_Client_V1_CreateAgentSessionRequest()
    request.provider = "claude"; request.workspaceIdentity = path; request.worktreePath = path
    request.rows = 24; request.cols = 80; request.callerRequestID = UUID().uuidString
    let selection = try await client.createAgentSession(request: request, headers: headers).result.get()
    var rootID = ""
    try await fixture.waitForState(target: "workspaces") { payload in
      guard case .workspaces(let value) = payload.value,
        let worktree = value.repositories.items.flatMap(\.worktrees.items).first(where: { $0.path == path }),
        let root = worktree.snapshot.nodes.items.first(where: {
          if case .node(let node) = $0.variant { return node.sessionCapabilities.sessionRef == selection.agentSessionID }
          return false
        }), case .node(let node) = root.variant else { return false }
      rootID = node.id
      return true
    }
    launch()
    let worktree = app.buttons["worktree:main"].firstMatch
    XCTAssertTrue(worktree.waitForExistence(timeout: 30), app.debugDescription); worktree.click()
    let panel = app.checkBoxes["Root の一覧"]
    XCTAssertTrue(panel.waitForExistence(timeout: 10), app.debugDescription); panel.click()
    let root = app.buttons["root:\(rootID)"]
    XCTAssertTrue(root.waitForExistence(timeout: 30), app.debugDescription); root.click()
    let terminal = app.textViews["terminal:\(selection.agentSessionID)"]
    XCTAssertTrue(terminal.waitForExistence(timeout: 30), app.debugDescription)
    let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value CONTAINS %@", "e2e-provider"), object: terminal)
    let readyResult = await XCTWaiter.fulfillment(of: [ready], timeout: 30)
    XCTAssertEqual(readyResult, .completed)
    terminal.click(); terminal.typeText("session-tab-input\n")
    let input = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value CONTAINS %@", "received-0:session-tab-input"), object: terminal)
    let inputResult = await XCTWaiter.fulfillment(of: [input], timeout: 30)
    XCTAssertEqual(inputResult, .completed)
  }

  @MainActor func testCommandTabShowsRootCommandOutputAndExitCode() async throws {
    try await fixture.startServer()
    let repository = try await fixture.repository("command-tab-repository")
    let (client, headers) = try fixture.client()
    var definition = Releash_Client_V1_SaveWorkflowSourceRequest()
    definition.source = """
      name: command-tab-e2e
      description: Command tab regression
      nodes:
        main:
          command: printf command-tab-output
      """
    let saved = try await client.saveWorkflowSource(request: definition, headers: headers).result.get()
    guard case .success = saved.variant else { XCTFail("Workflowを保存できません"); return }
    var start = Releash_Client_V1_StartWorkflowRequest()
    start.workflowName = "command-tab-e2e"; start.worktreePath = repository.path
    let execution = try await client.startWorkflow(request: start, headers: headers).result.get()
    launch()
    let worktree = app.buttons["worktree:main"].firstMatch
    XCTAssertTrue(worktree.waitForExistence(timeout: 30), app.debugDescription); worktree.click()
    let panel = app.checkBoxes["Root の一覧"]
    XCTAssertTrue(panel.waitForExistence(timeout: 10), app.debugDescription); panel.click()
    let root = app.buttons["root:\(execution.value)"]
    XCTAssertTrue(root.waitForExistence(timeout: 30), app.debugDescription); root.click()
    let output = app.staticTexts["command-stdout"]
    XCTAssertTrue(output.waitForExistence(timeout: 30), app.debugDescription)
    let received = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value CONTAINS %@", "command-tab-output"), object: output)
    let receivedResult = await XCTWaiter.fulfillment(of: [received], timeout: 30)
    XCTAssertEqual(receivedResult, .completed)
    let result = app.staticTexts["command-result"]
    XCTAssertTrue(result.waitForExistence(timeout: 30), app.debugDescription)
    XCTAssertTrue((result.value as? String)?.contains("Exit code: 0") == true, result.debugDescription)
  }
}

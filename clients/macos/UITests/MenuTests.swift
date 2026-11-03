import XCTest

extension WorkbenchTests {
  @MainActor func testFileMenuOpensRepositoryFolder() async throws {
    // Given
    try await fixture.startServer()
    let repository = fixture.directory.appendingPathComponent("opened-from-menu")
    try FileManager.default.createDirectory(at: repository, withIntermediateDirectories: true)
    try fixture.process(
      URL(fileURLWithPath: "/usr/bin/git"), ["init", "-b", "main"], at: repository)
    launch()
    // When
    app.menuBars.menuBarItems["File"].click()
    app.menuItems["フォルダを開く…"].click()
    let dialog = app.dialogs.firstMatch
    XCTAssertTrue(dialog.waitForExistence(timeout: 10), app.debugDescription)
    dialog.typeKey("g", modifierFlags: [.command, .shift])
    let path = app.textFields["PathTextField"]
    XCTAssertTrue(path.waitForExistence(timeout: 10), app.debugDescription)
    path.typeText(repository.path + "\n")
    let open = dialog.buttons["Open"]
    XCTAssertTrue(open.waitForExistence(timeout: 10), app.debugDescription)
    open.click()
    // Then
    let header = app.descendants(matching: .any).matching(
      NSPredicate(format: "identifier == %@", "repository:\(repository.path)")
    ).firstMatch
    XCTAssertTrue(header.waitForExistence(timeout: 30), app.debugDescription)
  }

  @MainActor func testGitMenuStagesAndUnstagesAllChanges() async throws {
    // Given
    try await fixture.startServer()
    let repository = try await fixture.repository("staging-menu")
    try "changed\n".write(
      to: repository.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    launch()
    app.buttons["worktree:main"].firstMatch.click()
    // When
    app.menuBars.menuBarItems["Git"].click()
    app.menuItems["すべてステージ"].click()
    // Then
    let staged = XCTNSPredicateExpectation(
      predicate: NSPredicate { [self] _, _ in
        let result = try? fixture.process(
          URL(fileURLWithPath: "/usr/bin/git"), ["diff", "--cached", "--name-only"], at: repository)
        return result.map { String(decoding: $0, as: UTF8.self) } == "file.txt\n"
      }, object: nil)
    XCTAssertEqual(XCTWaiter.wait(for: [staged], timeout: 30), .completed)
    // When
    app.menuBars.menuBarItems["Git"].click()
    app.menuItems["すべてアンステージ"].click()
    // Then
    let unstaged = XCTNSPredicateExpectation(
      predicate: NSPredicate { [self] _, _ in
        let result = try? fixture.process(
          URL(fileURLWithPath: "/usr/bin/git"), ["diff", "--cached", "--name-only"], at: repository)
        return result?.isEmpty == true
      }, object: nil)
    XCTAssertEqual(XCTWaiter.wait(for: [unstaged], timeout: 30), .completed)
    XCTAssertEqual(
      try String(contentsOf: repository.appendingPathComponent("file.txt"), encoding: .utf8),
      "changed\n")
  }

  @MainActor func testGitMenuCreatesAndChecksOutBranch() async throws {
    // Given
    try await fixture.startServer()
    _ = try await fixture.repository("menu-repository")
    launch()
    let worktree = app.buttons["worktree:main"].firstMatch
    XCTAssertTrue(worktree.waitForExistence(timeout: 30), app.debugDescription)
    // When
    worktree.click()
    app.menuBars.menuBarItems["Git"].click()
    app.menuItems["ブランチを作成…"].click()
    let name = app.textFields["create-branch-name"]
    // Then
    XCTAssertTrue(name.waitForExistence(timeout: 10), app.debugDescription)
    // When
    name.click()
    name.typeText("feature/menu")
    app.buttons["作成"].click()
    // Then
    XCTAssertTrue(
      app.buttons["worktree:feature/menu"].firstMatch.waitForExistence(timeout: 30),
      app.debugDescription)
  }
}

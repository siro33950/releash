import XCTest

extension WorkbenchTests {
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

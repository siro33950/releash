import XCTest

final class PullRequestTests: XCTestCase {
  @MainActor func testPullRequestShowsTitleDescriptionCommentAndChecks() async throws {
    // Given
    let fixture = try Fixture(pullRequest: true)
    let app = XCUIApplication()
    defer {
      app.terminate()
      XCTAssertNoThrow(try fixture.cleanUp())
    }
    try await fixture.startServer()
    let repository = try await fixture.repository("pr-repository")
    try fixture.process(
      URL(fileURLWithPath: "/usr/bin/git"),
      ["remote", "add", "origin", "https://github.com/e2e/repository.git"], at: repository)
    app.launchEnvironment = fixture.environment
    app.launch()
    let worktree = app.buttons["worktree:main"].firstMatch
    XCTAssertTrue(worktree.waitForExistence(timeout: 30), app.debugDescription)
    worktree.click()
    let panel = app.checkBoxes["PR"]
    if !panel.exists {
      app.buttons["右パネルを開く"].click()
    }
    // When
    XCTAssertTrue(panel.waitForExistence(timeout: 10), app.debugDescription)
    panel.click()
    // Then
    for label in ["E2E pull request", "E2E description", "E2E comment", "E2E check"] {
      XCTAssertTrue(
        app.staticTexts[label].firstMatch.waitForExistence(timeout: 30), app.debugDescription)
    }
    let evidence = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
    evidence.name = "PR title, description, comment and checks"
    evidence.lifetime = .keepAlways
    add(evidence)
  }
}

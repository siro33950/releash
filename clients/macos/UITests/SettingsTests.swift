import XCTest

final class SettingsTests: XCTestCase {
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

  @MainActor func testTerminalStartupCommandPersistsAfterReopeningSettings() async throws {
    app.launch()
    XCTAssertTrue(app.buttons["Worktreeを作成"].waitForExistence(timeout: 30), app.debugDescription)
    app.typeKey(",", modifierFlags: .command)
    let field = app.textFields["settings-terminal-command"]
    XCTAssertTrue(field.waitForExistence(timeout: 10), app.debugDescription)
    let enabled = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "enabled == true"), object: field)
    let ready = await XCTWaiter.fulfillment(of: [enabled], timeout: 10)
    XCTAssertEqual(ready, .completed)
    let command = "echo settings-e2e"
    field.click()
    field.typeKey("a", modifierFlags: .command)
    field.typeText(command + "\n")
    try await fixture.waitForState(target: "terminal-startup-command") { payload in
      guard case .terminalStartupCommand(let value) = payload.value else { return false }
      return value.value == command
    }
    let settings = app.windows.containing(.textField, identifier: "settings-terminal-command")
      .element
    settings.buttons[XCUIIdentifierCloseWindow].click()
    XCTAssertTrue(field.waitForNonExistence(timeout: 10))
    app.typeKey(",", modifierFlags: .command)
    XCTAssertTrue(field.waitForExistence(timeout: 10), app.debugDescription)
    let restored = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "value == %@", command), object: field)
    let result = await XCTWaiter.fulfillment(of: [restored], timeout: 10)
    XCTAssertEqual(result, .completed)
  }
}

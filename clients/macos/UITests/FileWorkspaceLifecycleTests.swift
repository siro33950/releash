import XCTest

final class FileWorkspaceLifecycleTests: XCTestCase {
  private var fixture: Fixture!
  private var app: XCUIApplication!

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

  @MainActor private func launch() {
    app.launch()
    XCTAssertTrue(app.buttons["Worktreeを作成"].waitForExistence(timeout: 30), app.debugDescription)
  }

  @MainActor private func prepareFileWorkspace(_ name: String) async throws {
    try await fixture.startServer()
    let repository = try await fixture.repository(name)
    for file in ["first.rs", "second.rs"] {
      try "before\n".write(
        to: repository.appendingPathComponent(file), atomically: true, encoding: .utf8)
    }
    let git = URL(fileURLWithPath: "/usr/bin/git")
    try fixture.process(git, ["add", "."], at: repository)
    try fixture.process(
      git,
      [
        "-c", "user.name=Acceptance", "-c", "user.email=test@example.invalid", "-c",
        "commit.gpgsign=false", "commit", "-m", "files",
      ], at: repository)
    try "after\n".write(
      to: repository.appendingPathComponent("first.rs"), atomically: true, encoding: .utf8)
    launch()
    let card = app.buttons["worktree:main"].firstMatch
    XCTAssertTrue(card.waitForExistence(timeout: 30))
    card.click()
    app.checkBoxes["ファイル"].click()
    for file in ["first.rs", "second.rs"] {
      let row = app.descendants(matching: .any).matching(identifier: "file:\(file)").firstMatch
      XCTAssertTrue(row.waitForExistence(timeout: 10))
      row.click()
      XCTAssertTrue(app.buttons[file].firstMatch.waitForExistence(timeout: 10))
    }
  }

  @MainActor private func selectFirstFileTab() async throws {
    let tab = app.buttons["first.rs"].firstMatch
    let strip = app.scrollViews.containing(.button, identifier: "first.rs").firstMatch
    XCTAssertTrue(strip.waitForExistence(timeout: 10))
    strip.scroll(byDeltaX: 1000, deltaY: 0)
    let visible = XCTNSPredicateExpectation(
      predicate: NSPredicate { _, _ in strip.frame.contains(tab.frame) && tab.isHittable },
      object: nil)
    let visibleResult = await XCTWaiter.fulfillment(of: [visible], timeout: 10)
    XCTAssertEqual(visibleResult, .completed)
    guard visibleResult == .completed else {
      throw NSError(domain: "FileWorkspaceAcceptance", code: 1)
    }
    tab.click()
    let selected = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "selected == true"), object: tab)
    let selectedResult = await XCTWaiter.fulfillment(of: [selected], timeout: 10)
    XCTAssertEqual(selectedResult, .completed)
    guard selectedResult == .completed else {
      throw NSError(domain: "FileWorkspaceAcceptance", code: 2)
    }
  }

  @MainActor private func chooseTabMenu(_ title: String) {
    app.menuBars.menuBarItems["タブ"].click()
    app.menuItems[title].click()
  }

  @MainActor func test_タブメニュー_b011_b012_フォーカスのある領域を操作する() async throws {
    // Given
    try await prepareFileWorkspace("file-menu-next")
    for _ in 0..<2 {
      app.menuButtons["開く"].firstMatch.click()
      app.menuItems["Terminal"].click()
    }
    try await selectFirstFileTab()
    app.buttons["Terminal 1"].firstMatch.click()
    // When
    chooseTabMenu("次のタブ")
    // Then
    let selected = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "selected == true"),
      object: app.buttons["Terminal 2"].firstMatch)
    let result = await XCTWaiter.fulfillment(of: [selected], timeout: 10)
    XCTAssertEqual(result, .completed)
    XCTAssertTrue(app.buttons["first.rs"].firstMatch.isSelected)
    XCTAssertFalse(app.buttons["second.rs"].firstMatch.isSelected)
    // Given / When: move focus to the file area and close its selected tab.
    try await selectFirstFileTab()
    chooseTabMenu("タブを閉じる")
    // Then
    let closed = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "exists == false"), object: app.buttons["first.rs"].firstMatch)
    let closeResult = await XCTWaiter.fulfillment(of: [closed], timeout: 10)
    XCTAssertEqual(closeResult, .completed)
    XCTAssertTrue(app.buttons["second.rs"].firstMatch.exists)
    XCTAssertTrue(app.buttons["Terminal 1"].firstMatch.exists)
    XCTAssertTrue(app.buttons["Terminal 2"].firstMatch.exists)
  }

  @MainActor func test_ファイル復元_b013_再起動後も分割と選択とbase比較が戻る() async throws {
    // Given
    try await prepareFileWorkspace("file-relaunch")
    let first = app.buttons["first.rs"].firstMatch
    let second = app.buttons["second.rs"].firstMatch
    try await selectFirstFileTab()
    let content = app.descendants(matching: .any).matching(identifier: "file-content").firstMatch
    XCTAssertTrue(content.waitForExistence(timeout: 10))
    first.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).click(
      forDuration: 0.5,
      thenDragTo: content.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.98)))
    let sharedStrip = app.scrollViews.containing(.button, identifier: "first.rs")
      .containing(.button, identifier: "second.rs").firstMatch
    let split = XCTNSPredicateExpectation(
      predicate: NSPredicate { _, _ in
        first.exists && second.exists && first.isSelected && second.isSelected
          && !sharedStrip.exists
      }, object: nil)
    let splitResult = await XCTWaiter.fulfillment(of: [split], timeout: 10)
    XCTAssertEqual(splitResult, .completed)
    func baseComparison() throws -> XCUIElement {
      let pane = try XCTUnwrap(
        app.groups.containing(.button, identifier: "first.rs")
          .containing(.checkBox, identifier: "base と比較").allElementsBoundByIndex
          .min { $0.frame.width * $0.frame.height < $1.frame.width * $1.frame.height })
      return pane.checkBoxes["base と比較"].firstMatch
    }
    let base = try baseComparison()
    base.click()
    XCTAssertEqual((base.value as? NSNumber)?.boolValue, true)
    let offset = CGVector(
      dx: first.frame.minX - second.frame.minX, dy: first.frame.minY - second.frame.minY)
    // When
    app.terminate()
    launch()
    app.buttons["worktree:main"].firstMatch.click()
    app.checkBoxes["ファイル"].click()
    // Then
    XCTAssertTrue(first.waitForExistence(timeout: 10))
    XCTAssertTrue(second.waitForExistence(timeout: 10))
    XCTAssertTrue(first.isSelected)
    XCTAssertTrue(second.isSelected)
    XCTAssertFalse(sharedStrip.exists)
    XCTAssertEqual(first.frame.minX - second.frame.minX, offset.dx, accuracy: 3)
    XCTAssertEqual(first.frame.minY - second.frame.minY, offset.dy, accuracy: 3)
    let restoredBase = try baseComparison()
    XCTAssertEqual((restoredBase.value as? NSNumber)?.boolValue, true)
  }
}

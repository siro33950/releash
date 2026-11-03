import ApplicationServices
import SwiftUI
import XCTest

@testable import Releash

final class CreateBranchViewTests: XCTestCase {
  @MainActor func testBranchInputSubmitsNameAndDisablesUnavailableActions() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectedWorktree = "/repo/linked"
    // When
    let host = await ViewHarness(CreateBranchView(model: model))
    defer { host.close() }
    // Then
    try await eventuallyView(host) { host.element("作成")?.enabled == false }
    // When
    try await host.focusTextField()
    try await host.setTextField("feature/menu")
    // Then
    try await eventuallyView(host) { host.element("作成")?.enabled == true }
    // When
    model.connected = false
    // Then
    try await eventuallyView(host) { host.element("作成")?.enabled == false }
    // When
    model.connected = true
    model.selectedWorktree = nil
    // Then
    try await eventuallyView(host) { host.element("作成")?.enabled == false }
    // When
    model.selectedWorktree = "/repo/linked"
    // Then
    try await eventuallyView(host) { host.element("作成")?.enabled == true }
    // When
    try await host.press("作成")
    // Then
    try await eventually { http.requests("GitCreateBranch").count == 1 }
    let request = try Releash_Client_V1_GitCreateBranchRequest(
      serializedBytes: XCTUnwrap(http.requests("GitCreateBranch").first?.message))
    XCTAssertEqual(request.repoPath, "/repo/linked")
    XCTAssertEqual(request.branchName, "feature/menu")
    await model.stop()
  }

  @MainActor func testCancelDoesNotCreateBranch() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectedWorktree = "/repo/linked"
    let host = await ViewHarness(RootView(model: model).overlay { BranchMenuButton() })
    defer { host.close() }
    try await makeKey(host)
    try await host.press("open-branch-sheet")
    try await eventuallyView(host) { host.element("create-branch-name") != nil }
    // When
    try await host.press("キャンセル")
    // Then
    try await eventually { host.window.attachedSheet == nil }
    XCTAssertTrue(http.requests("GitCreateBranch").isEmpty)
    await model.stop()
  }

  @MainActor func testOnlyKeyWindowPresentsBranchSheetAndCancelIsIndependent() async throws {
    // Given
    let model = try await testModel(TestHTTPClient())
    model.selectedWorktree = "/repo/linked"
    let first = await ViewHarness(RootView(model: model).overlay { BranchMenuButton() })
    defer { first.close() }
    let second = await ViewHarness(RootView(model: model).overlay { BranchMenuButton() })
    defer { second.close() }
    // When
    try await makeKey(first)
    try await first.press("open-branch-sheet")
    // Then
    try await eventuallyView(first) { first.element("create-branch-name") != nil }
    XCTAssertNil(second.window.attachedSheet)
    // When
    try focusSheet(first)
    try await first.setTextField("feature/first")
    try await makeKey(second)
    try await second.press("open-branch-sheet")
    // Then
    try await eventuallyView(second) { second.element("create-branch-name") != nil }
    // When
    try await second.press("キャンセル")
    // Then
    try await eventually { second.window.attachedSheet == nil }
    XCTAssertNotNil(first.window.attachedSheet)
    try await eventuallyView(first) {
      first.value(first.element("create-branch-name")) == "feature/first"
    }
    // When
    try await first.press("キャンセル")
    // Then
    try await eventually { first.window.attachedSheet == nil }
    await model.stop()
  }

  @MainActor func testBranchCommandIsDisabledOutsideWorkspaceWindow() async throws {
    // Given
    let host = await ViewHarness(BranchMenuButton())
    defer { host.close() }
    // When
    try await makeKey(host)
    // Then
    try await eventuallyView(host) { host.element("open-branch-sheet")?.enabled == false }
  }

  @MainActor func testFailedCreationKeepsNameAndSheetUntilSuccessfulRetry() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.failedMethod = "GitCreateBranch" }
    let model = try await testModel(http)
    model.selectedWorktree = "/repo/linked"
    let host = await ViewHarness(RootView(model: model).overlay { BranchMenuButton() })
    defer { host.close() }
    // When
    try await makeKey(host)
    try await host.press("open-branch-sheet")
    // Then
    try await eventuallyView(host) { host.element("create-branch-name") != nil }
    // When
    try focusSheet(host)
    try await host.setTextField("feature/menu")
    // Then
    try await eventuallyView(host) { host.element("作成")?.enabled == true }
    // When
    try await host.press("作成")
    // Then
    try await eventuallyView(host) { host.texts().contains("GitCreateBranch failed") }
    XCTAssertNotNil(host.window.attachedSheet)
    XCTAssertEqual(host.value(host.element("create-branch-name")), "feature/menu")
    XCTAssertNil(model.failure)
    // Given
    http.state.withLock { $0.failedMethod = nil }
    try await eventuallyView(host) { host.element("作成")?.enabled == true }
    // When
    try await host.press("作成")
    // Then
    try await eventually { host.window.attachedSheet == nil }
    XCTAssertEqual(http.requests("GitCreateBranch").count, 2)
    await model.stop()
  }

  @MainActor private func focusSheet(_ host: ViewHarness) throws {
    func fields(_ view: NSView) -> [NSTextField] {
      (view as? NSTextField).map { [$0] } ?? view.subviews.flatMap(fields)
    }
    let sheet = try XCTUnwrap(host.window.attachedSheet)
    let field = try XCTUnwrap(fields(try XCTUnwrap(sheet.contentView)).first { $0.isEditable })
    sheet.makeKeyAndOrderFront(nil)
    XCTAssertTrue(sheet.makeFirstResponder(field))
  }

  @MainActor private func makeKey(_ host: ViewHarness) async throws {
    let application = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
    XCTAssertEqual(
      AXUIElementSetAttributeValue(application, kAXFrontmostAttribute as CFString, kCFBooleanTrue),
      .success)
    XCTAssertTrue(host.window.canBecomeKey)
    try await eventually {
      host.window.makeKeyAndOrderFront(nil)
      return NSApp.keyWindow === host.window
    }
  }
}

private struct BranchMenuButton: View {
  @FocusedValue(\.createBranch) private var createBranch

  var body: some View {
    Button("Open branch sheet") { createBranch?() }
      .disabled(createBranch == nil)
      .accessibilityIdentifier("open-branch-sheet")
  }
}

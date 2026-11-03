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
    model.creatingBranch = true
    let host = await ViewHarness(RootView(model: model))
    defer { host.close() }
    try await eventuallyView(host) { host.element("create-branch-name") != nil }
    // When
    try await host.press("キャンセル")
    // Then
    try await eventually { !model.creatingBranch }
    XCTAssertTrue(http.requests("GitCreateBranch").isEmpty)
    await model.stop()
  }
}

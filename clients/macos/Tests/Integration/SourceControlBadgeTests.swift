import SwiftUI
import XCTest

@testable import Releash

final class SourceControlBadgeTests: XCTestCase {
  @MainActor func test_変更数バッジ_境界値と取得失敗後の回復を表示する() async throws {
    // Given
    let model = AppModel()
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.path = "/repo"
    repository.branches.items = [
      .with {
        $0.worktreePath = "/repo/feature"
        $0.dirtyCount = 99
      }
    ]
    model.repositories = [repository]
    model.selectedWorktree = "/repo/feature"
    let host = await ViewHarness(RightPanelView(expanded: .constant(true), model: model))
    defer { host.close() }
    // When / Then
    try await eventuallyView(host) { host.element("99") != nil }
    model.repositories[0].branches.items[0].dirtyCount = 100
    try await eventuallyView(host) { host.element("99+") != nil && host.element("99") == nil }
    let view = try XCTUnwrap(host.window.contentView)
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
    view.cacheDisplay(in: view.bounds, to: bitmap)
    let image = NSImage(size: bitmap.size)
    image.addRepresentation(bitmap)
    let attachment = XCTAttachment(image: image)
    attachment.name = "Source Control 99+"
    attachment.lifetime = .keepAlways
    add(attachment)
    model.repositories[0].branches.items[0].dirtyCountError = "Unavailable"
    try await eventuallyView(host) { host.element("99+") == nil }
    model.repositories[0].branches.items[0].clearDirtyCountError()
    model.repositories[0].branches.items[0].dirtyCount = 2
    try await eventuallyView(host) { host.element("2") != nil }
    model.repositories = []
    try await eventuallyView(host) { host.element("2") == nil }
    model.repositories = [repository]
    try await eventuallyView(host) { host.element("99") != nil }
    model.selectedWorktree = "/repo/missing"
    try await eventuallyView(host) { host.element("99") == nil }
  }
}

import AppKit
import ApplicationServices
import Bonsplit
import SwiftUI
import XCTest

@testable import Releash

final class FileTabPaneTests: XCTestCase {
  @MainActor func pane(_ model: AppModel, value: Releash_Client_V1_FileTabView? = nil) throws -> FileTabPane {
    try model.panes.restore(.empty())
    model.panes.openFile("src/file.rs", base: "head")
    return FileTabPane(model: model, tabID: try XCTUnwrap(model.panes.controller.allTabIds.first),
      path: "src/file.rs", worktree: "/repo", value: value)
  }

  @MainActor func testToolbarShowsModeSpecificItemsAndFitsDefaultPaneWidthWithLongPath() async throws {
    let common = ["file-path", "file-mode", "file-thread-count", "file-open-editor"]
    let diffOnly = ["file-base", "file-stats", "file-style", "file-full"]
    for mode in ["file", "diff"] {
      let model = AppModel()
      let original = try pane(model)
      original.setPresentation(mode: mode)
      let path = String(repeating: "long-directory/", count: 30) + "file.rs"
      var value = Releash_Client_V1_FileTabView(); value.additions = 12; value.deletions = 34
      let view = FileTabPane(model: model, tabID: original.tabID, path: path, worktree: "/repo", value: value)
      let width = UIStyle.windowWidth - 2 * UIStyle.windowPadding - 2 * UIStyle.sidebarWidth - 2 * UIStyle.dividerWidth
      let host = NSHostingView(rootView: view.toolbar)
      host.sizingOptions = []
      let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 44), styleMask: [.borderless], backing: .buffered, defer: false)
      window.isReleasedWhenClosed = false
      window.contentView = host
      window.orderFront(nil)
      defer { window.close() }
      host.layoutSubtreeIfNeeded()
      window.displayIfNeeded()
      let requestResult = await Task.detached {
        let application = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
        var windows: CFTypeRef?
        return AXUIElementCopyAttributeValue(application, kAXWindowsAttribute as CFString, &windows).rawValue
      }.value
      XCTAssertEqual(requestResult, AXError.success.rawValue)
      var elements: [String: AnyObject] = [:]
      var visited = Set<ObjectIdentifier>()
      var scrollAreas = 0
      func collect(_ value: Any) {
        let element = value as AnyObject
        guard visited.insert(ObjectIdentifier(element)).inserted else { return }
        if let id = element.accessibilityIdentifier?() { elements[id] = element }
        if element.accessibilityRole?() == .scrollArea { scrollAreas += 1 }
        for child in element.accessibilityChildren?() ?? [] { collect(child) }
      }
      try await eventually {
        elements.removeAll(); visited.removeAll(); scrollAreas = 0; collect(host)
        return common.allSatisfy { elements[$0] != nil }
      }
      XCTAssertEqual(scrollAreas, 0)
      XCTAssertEqual(Set(elements.keys).intersection(Set(common + diffOnly)), Set(common + (mode == "diff" ? diffOnly : [])))
      XCTAssertLessThanOrEqual(host.bounds.width, width)
      let visible = window.convertToScreen(host.bounds).insetBy(dx: -1, dy: -1)
      for id in common + (mode == "diff" ? diffOnly : []) {
        let element = try XCTUnwrap(elements[id], id)
        let frame = try XCTUnwrap(element.accessibilityFrame?())
        XCTAssertGreaterThan(frame.width, 0, id)
        XCTAssertGreaterThan(frame.height, 0, id)
        XCTAssertTrue(visible.contains(frame), "\(mode): \(id) \(frame) outside \(visible)")
      }
      let fullPathWidth = NSAttributedString(string: path, attributes: [.font: NSFont.systemFont(ofSize: NSFont.systemFontSize)]).size().width
      let pathElement = try XCTUnwrap(elements["file-path"])
      XCTAssertLessThan(try XCTUnwrap(pathElement.accessibilityFrame?()).width, fullPathWidth)
    }
  }

  @MainActor func testDisplaySettingsReachAllTabsAndSurviveNewAppState() async throws {
    let defaults = UserDefaults.standard
    let keys = ["diffStyle", "diffFull", "diffBase"]
    let saved = keys.map { defaults.object(forKey: $0) }
    defer { for (key, value) in zip(keys, saved) { defaults.set(value, forKey: key) } }
    for key in keys { defaults.removeObject(forKey: key) }
    let model = AppModel()
    let first = try pane(model)
    model.openFile("default.rs")
    XCTAssertEqual(model.panes.tab(try XCTUnwrap(model.panes.controller.allTabIds.last))?.fileBase, "head")
    defaults.set("branch-base", forKey: "diffBase")
    model.openFile("base.rs")
    XCTAssertEqual(model.panes.tab(try XCTUnwrap(model.panes.controller.allTabIds.last))?.fileBase, "branch-base")
    let second = try pane(AppModel())
    var text = Releash_Client_V1_ReviewTextDiffDto()
    text.original = "old\n"; text.modified = "new\n"
    var value = Releash_Client_V1_FileTabView(); value.patch = "server patch"
    first.settings.style = "unified"; first.settings.full = true
    var payload = second.bridgePayload(text, value, colorScheme: .dark)
    XCTAssertEqual(payload["style"] as? String, "unified")
    XCTAssertEqual(payload["full"] as? Bool, true)
    XCTAssertEqual(payload["dark"] as? Bool, true)
    let restarted = AppModel()
    let reopened = try pane(restarted)
    payload = reopened.bridgePayload(text, value, colorScheme: .light)
    XCTAssertEqual(payload["style"] as? String, "unified")
    XCTAssertEqual(payload["full"] as? Bool, true)
    XCTAssertEqual(payload["dark"] as? Bool, false)
    restarted.openFile("new.rs")
    XCTAssertEqual(restarted.panes.tab(try XCTUnwrap(restarted.panes.controller.allTabIds.last))?.fileBase, "branch-base")
    second.settings.style = "split"; second.settings.full = false
    var liveStyle = ""
    var liveFull = true
    let host = NSHostingView(rootView: SettingsProbe { style, full in liveStyle = style; liveFull = full })
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 100, height: 100), styleMask: [.borderless], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.contentView = host
    defer { window.close() }
    host.layoutSubtreeIfNeeded()
    try await eventually { liveStyle == "split" && !liveFull }
    first.settings.style = "unified"; first.settings.full = true
    try await eventually { liveStyle == "unified" && liveFull }
    second.settings.style = "split"; second.settings.full = false
    try await eventually { liveStyle == "split" && !liveFull }
  }

  @MainActor func testThreadAnnotationsRetainSideAndServerResolution() throws {
    for side in [Releash_Client_V1_ReviewLineSideDto.Value.deletions, .additions] {
      var thread = Releash_Client_V1_ReviewThreadDto()
      thread.id = "thread"; thread.target.lineNumber = 2; thread.target.side.value = side
      thread.canResolve = true
      thread.comments.items = [.with { $0.author.displayName = "human"; $0.content = "body" }]
      var threads = Releash_Client_V1_FileThreadsSnapshot(); threads.threads.items = [thread]
      let model = AppModel()
      let base = try pane(model)
      let view = FileTabPane(model: model, tabID: base.tabID, path: base.path, worktree: base.worktree, threads: threads)
      for resolved in [false, true] {
        threads.threads.items[0].state.value = resolved ? .resolved : .open
        let snapshot = FileTabPane(model: model, tabID: view.tabID, path: view.path, worktree: view.worktree, threads: threads)
        let payload = snapshot.bridgePayload(.init(), .init(), colorScheme: .light)
        let annotations = try XCTUnwrap(payload["threads"] as? [[String: Any]])
        XCTAssertEqual(annotations[0]["side"] as? String, side == .deletions ? "deletions" : "additions")
        XCTAssertEqual(annotations[0]["line"] as? UInt32, 2)
        XCTAssertEqual(annotations[0]["resolved"] as? Bool, resolved)
        XCTAssertEqual(annotations[0]["canResolve"] as? Bool, true)
        XCTAssertEqual((annotations[0]["comments"] as? [[String: String]])?.first?["content"], "body")
      }
    }
  }

  @MainActor func testFileActionsSendTargetAndThreadInputAndReportRPCFailure() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let view = try pane(model)
    view.openInEditor()
    var acceptedReplies = 0
    view.action(["kind": "reply", "id": "thread", "content": "reply body"]) { acceptedReplies += 1 }
    view.action(["kind": "resolve", "id": "thread"])
    view.action(["kind": "stage", "id": "hunk"])
    view.action(["kind": "unstage", "id": "hunk"])
    view.action(["kind": "create", "line": NSNumber(value: 2), "endLine": NSNumber(value: 3), "side": "deletions", "endSide": "deletions"])
    XCTAssertEqual(view.draft.line, 2)
    XCTAssertEqual(view.draft.side, .deletions)
    view.draft.content = "selected line"
    view.createThread()
    try await eventually { http.requests("CreateReviewThread").count == 1 && http.requests("ResolveReviewThread").count == 1
      && http.requests("AppendReviewComment").count == 1 && http.requests("OpenInEditor").count == 1
      && http.requests("GitStageReviewGroup").count == 1 && http.requests("GitUnstageReviewGroup").count == 1 && acceptedReplies == 1 }
    let open = try Releash_Client_V1_OpenInEditorRequest(serializedBytes: XCTUnwrap(http.requests("OpenInEditor").first?.message))
    XCTAssertEqual(open.filePath, "/repo/src/file.rs")
    XCTAssertEqual(acceptedReplies, 1)
    let reply = try Releash_Client_V1_AppendReviewCommentRequest(serializedBytes: XCTUnwrap(http.requests("AppendReviewComment").first?.message))
    XCTAssertEqual(reply.worktreeName, "/repo"); XCTAssertEqual(reply.threadID, "thread"); XCTAssertEqual(reply.content, "reply body")
    let resolve = try Releash_Client_V1_ResolveReviewThreadRequest(serializedBytes: XCTUnwrap(http.requests("ResolveReviewThread").first?.message))
    XCTAssertEqual(resolve.threadID, "thread"); XCTAssertEqual(resolve.worktreeName, "/repo"); XCTAssertEqual(resolve.outcome, "resolved")
    let stage = try Releash_Client_V1_GitStageReviewGroupRequest(serializedBytes: XCTUnwrap(http.requests("GitStageReviewGroup").first?.message))
    let unstage = try Releash_Client_V1_GitUnstageReviewGroupRequest(serializedBytes: XCTUnwrap(http.requests("GitUnstageReviewGroup").first?.message))
    XCTAssertEqual(stage.input, unstage.input)
    XCTAssertEqual(stage.input.worktreePath, "/repo"); XCTAssertEqual(stage.input.path, "src/file.rs")
    XCTAssertEqual(stage.input.section, "file-tab"); XCTAssertEqual(stage.input.base, "head"); XCTAssertEqual(stage.input.groupID, "hunk")
    let create = try Releash_Client_V1_CreateReviewThreadRequest(serializedBytes: XCTUnwrap(http.requests("CreateReviewThread").first?.message))
    XCTAssertEqual(create.worktreeName, "/repo"); XCTAssertEqual(create.filePath, "src/file.rs")
    XCTAssertEqual(create.lineNumber, 2); XCTAssertEqual(create.endLine, 3); XCTAssertEqual(create.content, "selected line")
    XCTAssertEqual(create.side.value, .deletions); XCTAssertEqual(create.endSide.value, .deletions)
    XCTAssertNil(view.draft.line)
    view.action(["kind": "create", "line": NSNumber(value: 4), "endLine": NSNumber(value: 4), "side": "additions", "endSide": "additions"])
    XCTAssertEqual(view.draft.side, .additions)
    XCTAssertEqual(view.draft.content, "")
    view.draft.content = "added line"
    view.createThread()
    try await eventually { http.requests("CreateReviewThread").count == 2 && view.draft.line == nil }
    let added = try Releash_Client_V1_CreateReviewThreadRequest(serializedBytes: XCTUnwrap(http.requests("CreateReviewThread").last?.message))
    XCTAssertEqual(added.side.value, .additions); XCTAssertEqual(added.lineNumber, 4)
    http.state.withLock { $0.failedMethod = "AppendReviewComment" }
    view.action(["kind": "reply", "id": "thread", "content": "failure"]) { acceptedReplies += 1 }
    try await eventually { model.failure != nil }
    XCTAssertTrue(try XCTUnwrap(model.failure).contains("AppendReviewComment failed"))
    XCTAssertEqual(acceptedReplies, 1)
    await model.stop()
  }

  @MainActor func testImagesUseOnlyPresentSidesAndBinaryShowsReasonAndSize() throws {
    for (original, modified, initial, expected) in [
      (true, true, "diff", ["変更前の画像", "作業ツリーの画像"]),
      (false, true, "diff", ["作業ツリーの画像"]),
      (true, false, "diff", ["変更前の画像"]),
      (true, true, "file", ["作業ツリーの画像"]),
    ] {
      var image = Releash_Client_V1_ReviewImageDto()
      if original { image.originalURL = "data:image/png;base64,AQID" }
      if modified { image.modifiedURL = "data:image/png;base64,BAUG" }
      var value = Releash_Client_V1_FileTabView(); value.initialMode = initial; value.view.image = image
      let view = try pane(AppModel(), value: value)
      XCTAssertEqual(view.imageURLs(image, initialMode: initial).map(\.label), expected)
      XCTAssertEqual(view.imageURLs(image, initialMode: initial).map(\.url), expected.map { $0 == "変更前の画像" ? image.originalURL : image.modifiedURL })
      view.setPresentation(mode: "file")
      XCTAssertEqual(view.imageURLs(image, initialMode: initial).count, modified ? 1 : 0)
    }
    let view = try pane(AppModel())
    var binary = Releash_Client_V1_ReviewBinaryDto(); binary.originalSize = 10; binary.modifiedSize = 20
    XCTAssertEqual(view.binaryDescription(binary), ["画像以外のバイナリは表示できません", "20 bytes"])
    binary.clearModifiedSize()
    XCTAssertEqual(view.binaryDescription(binary), ["画像以外のバイナリは表示できません", "10 bytes"])
    binary.modifiedSize = 0
    XCTAssertEqual(view.binaryDescription(binary).last, "0 bytes")
  }
}

private struct SettingsProbe: View {
  var settings = FileDisplaySettings()
  var receive: (String, Bool) -> Void
  var body: some View {
    Text(settings.style)
      .onChange(of: settings.style, initial: true) { _, value in receive(value, settings.full) }
      .onChange(of: settings.full, initial: true) { _, value in receive(settings.style, value) }
  }
}

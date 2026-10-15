import AppKit
import ApplicationServices
import Bonsplit
import SwiftUI
import XCTest

@testable import Releash

final class FileTabPaneTests: XCTestCase {
  @MainActor func pane(_ model: AppModel, value: Releash_Client_V1_FileTabView? = nil) throws
    -> FileTabPane
  {
    try model.panes.restore(.empty())
    model.panes.openFile("src/file.rs", base: "head")
    return FileTabPane(
      model: model, tabID: try XCTUnwrap(model.panes.controller.allTabIds.first),
      path: "src/file.rs", worktree: "/repo", value: value)
  }
  @MainActor func testToolbarShowsModeSpecificItemsAndFitsDefaultPaneWidthWithLongPath()
    async throws
  {
    let common = ["file-path", "file-mode", "file-open-editor"]
    let diffOnly = ["file-base", "file-style", "file-full"]
    for mode in ["file", "diff"] {
      let model = AppModel()
      let original = try pane(model)
      original.setPresentation(mode: mode)
      let path = String(repeating: "long-directory/", count: 30) + "file.rs"
      let value = Releash_Client_V1_FileTabView()
      let view = FileTabPane(
        model: model, tabID: original.tabID, path: path, worktree: "/repo", value: value)
      let width =
        UIStyle.windowWidth - 2 * UIStyle.windowPadding - 2 * UIStyle.sidebarWidth - 2
        * UIStyle.dividerWidth
      let host = NSHostingView(rootView: view.toolbar)
      host.sizingOptions = []
      let window = NSWindow(
        contentRect: NSRect(x: 0, y: 0, width: width, height: 44), styleMask: [.borderless],
        backing: .buffered, defer: false)
      window.isReleasedWhenClosed = false
      window.contentView = host
      window.orderFront(nil)
      defer { window.close() }
      host.layoutSubtreeIfNeeded()
      window.displayIfNeeded()
      let requestResult = await Task.detached {
        let application = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
        var windows: CFTypeRef?
        return AXUIElementCopyAttributeValue(application, kAXWindowsAttribute as CFString, &windows)
          .rawValue
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
        elements.removeAll()
        visited.removeAll()
        scrollAreas = 0
        collect(host)
        return common.allSatisfy { elements[$0] != nil }
      }
      XCTAssertEqual(scrollAreas, 0)
      XCTAssertEqual(
        Set(elements.keys).intersection(Set(common + diffOnly)),
        Set(common + (mode == "diff" ? diffOnly : [])))
      XCTAssertLessThanOrEqual(host.bounds.width, width)
      let visible = window.convertToScreen(host.bounds).insetBy(dx: -1, dy: -1)
      for id in common + (mode == "diff" ? diffOnly : []) {
        let element = try XCTUnwrap(elements[id], id)
        let frame = try XCTUnwrap(element.accessibilityFrame?())
        XCTAssertGreaterThan(frame.width, 0, id)
        XCTAssertGreaterThan(frame.height, 0, id)
        XCTAssertTrue(visible.contains(frame), "\(mode): \(id) \(frame) outside \(visible)")
      }
      let fullPathWidth = NSAttributedString(
        string: path, attributes: [.font: NSFont.systemFont(ofSize: NSFont.systemFontSize)]
      ).size().width
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
    XCTAssertEqual(
      model.panes.tab(try XCTUnwrap(model.panes.controller.allTabIds.last))?.fileBase, "head")
    defaults.set("branch-base", forKey: "diffBase")
    model.openFile("base.rs")
    XCTAssertEqual(
      model.panes.tab(try XCTUnwrap(model.panes.controller.allTabIds.last))?.fileBase, "branch-base"
    )
    let second = try pane(AppModel())
    var text = Releash_Client_V1_ReviewTextDiffDto()
    text.original = "old\n"
    text.modified = "new\n"
    var value = Releash_Client_V1_FileTabView()
    value.patch = "server patch"
    first.settings.style = "unified"
    first.settings.full = true
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
    XCTAssertEqual(
      restarted.panes.tab(try XCTUnwrap(restarted.panes.controller.allTabIds.last))?.fileBase,
      "branch-base")
    second.settings.style = "split"
    second.settings.full = false
    var liveStyle = ""
    var liveFull = true
    let host = NSHostingView(
      rootView: SettingsProbe { style, full in
        liveStyle = style
        liveFull = full
      })
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 100, height: 100), styleMask: [.borderless],
      backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.contentView = host
    defer { window.close() }
    host.layoutSubtreeIfNeeded()
    try await eventually { liveStyle == "split" && !liveFull }
    first.settings.style = "unified"
    first.settings.full = true
    try await eventually { liveStyle == "unified" && liveFull }
    second.settings.style = "split"
    second.settings.full = false
    try await eventually { liveStyle == "split" && !liveFull }
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

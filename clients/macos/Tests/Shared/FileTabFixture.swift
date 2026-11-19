import AppKit
import ApplicationServices
import Bonsplit
import SwiftUI
import WebKit
import XCTest

@testable import Releash

extension FileViewerAcceptanceFixture where Self: XCTestCase {
  func press(_ host: NSWindow, _ name: String) async throws {
    try await eventually { self.findElement(host, name: name) != nil }
    let target = try XCTUnwrap(findElement(host, name: name))
    XCTAssertEqual(AXUIElementPerformAction(target, kAXPressAction as CFString), .success)
  }

  func selectedPath(_ name: String, in host: NSWindow) async throws {
    try await eventually {
      guard let element = self.findElement(host, name: "file-path") else { return false }
      var value: CFTypeRef?
      return AXUIElementCopyAttributeValue(element, kAXValueAttribute as CFString, &value)
        == .success && value as? String == name
    }
  }

  func fileLayout(_ names: [String], mode: String = "file") -> PaneLayout {
    let tabs = names.map { PaneTab(kind: .file, filePath: $0, fileMode: mode, fileBase: "head") }
    return .pane(id: UUID().uuidString, tabs: tabs, active: tabs.last?.id)
  }

  func filesPanel(_ layout: PaneLayout) async throws -> (
    TestHTTPClient, AppModel, NSWindow
  ) {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    http.state.withLock { state in
      for path in layout.tabs.compactMap(\.filePath) {
        state.fileThreads[["/repo", path]] = .init()
      }
    }
    try model.filePanes.restore(layout)
    model.saveFilePaneLayout()
    let host = window(RightPanelView(expanded: .constant(true), model: model))
    addTeardownBlock { @MainActor in
      host.close()
      await model.stop()
    }
    return (http, model, host)
  }

  func subscriptions(_ http: TestHTTPClient)
    -> [Releash_Client_V1_StartStateSubscriptionRequest]
  {
    http.state.withLock { state in
      state.subscriptions.filter {
        ["file-tab", "file-threads"].contains($0.target) && $0.args.first == "/repo"
      }
    }
  }

  func stoppedSubscriptionIDs(_ http: TestHTTPClient) -> Set<String> {
    Set(
      http.requests("StopStateSubscription").compactMap {
        try? Releash_Client_V1_StopStateSubscriptionRequest(serializedBytes: $0.message ?? Data())
          .subscriptionID
      })
  }

  func expectStarts(
    _ http: TestHTTPClient, in host: NSWindow, paths: [String], after count: Int = 0
  )
    async throws
  {
    let expected = Set(paths.flatMap { ["file-tab/" + $0, "file-threads/" + $0] })
    func received() -> Set<String> {
      Set(
        subscriptions(http).dropFirst(count).map {
          $0.target + "/" + ($0.args.dropFirst().first ?? "")
        })
    }
    try await eventually { expected.isSubset(of: received()) }
    for (index, path) in paths.enumerated() {
      var payload = Releash_Client_V1_StatePayload()
      payload.fileTab.view.binary.modifiedSize = UInt64(count + index + 100)
      try http.publish(payload, target: "file-tab", args: ["/repo", path, "head"])
      try await eventually {
        self.findElement(host, name: "\(count + index + 100) bytes") != nil
      }
    }
    XCTAssertEqual(received(), expected)
  }

  func snapshot(_ modified: String, mode: String) -> Releash_Client_V1_StatePayload {
    var payload = Releash_Client_V1_StatePayload()
    payload.fileTab.initialMode = mode
    payload.fileTab.view.textDiff.path = "acceptance.rs"
    payload.fileTab.view.textDiff.original = "one\nold\nthree\nfour\n"
    payload.fileTab.view.textDiff.modified = modified
    return payload
  }

  func viewer(_ host: NSWindow) async throws -> WKWebView {
    let content = try XCTUnwrap(host.contentView)
    func current() -> WKWebView? {
      views(content, WKWebView.self).first {
        !$0.isHiddenOrHasHiddenAncestor && !$0.visibleRect.isEmpty
      }
    }
    try await eventually { current() != nil }
    let web = try XCTUnwrap(current())
    web.configuration.preferences.inactiveSchedulingPolicy = .none
    return web
  }
}

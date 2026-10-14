import AppKit
import ApplicationServices
import SwiftUI
import Connect
import Foundation
import SwiftProtobuf
import Synchronization
import XCTest

@testable import Releash

final class TestHTTPClient: HTTPClientInterface {
  struct State {
    var requests: [HTTPRequest<Data?>] = []
    var streams: [ResponseCallbacks] = []
    var subscriptions: [Releash_Client_V1_StartStateSubscriptionRequest] = []
    var repositories: [Releash_Client_V1_WorkspaceRepositoryList] = []
    var layouts: [String: Releash_Client_V1_WorkspaceStateDto] = [:]
    var providerUsage = Releash_Client_V1_ProviderUsageSnapshot()
    var hookHealth = Releash_Client_V1_ProviderHookHealthSnapshot()
    var daemonInfo = Releash_Client_V1_ServerInfo()
    var collapsed: [String: Bool] = [:]
    var terminalReplay = ""
    var nodeDetails: [String: Releash_Client_V1_WorkspaceNodeDetailDto] = [:]
    var sessions: [String: Releash_Client_V1_AgentSessionItemDto] = [:]
    var history = Releash_Client_V1_AgentSessionHistoryPageDto()
    var failedMethod: String?
    var failureDetail: ConnectError.Detail?
    var failedSubscriptionTargets: [String: Int] = [:]
    var pendingCreation: (@Sendable (HTTPResponse) -> Void)?
    var deferCreation = false
    var deferSave = false
    var pendingSave: (@Sendable (HTTPResponse) -> Void)?
  }
  let state = Mutex(State())
  let discoveryFile = FileManager.default.temporaryDirectory.appendingPathComponent(
    UUID().uuidString)
  deinit { try? FileManager.default.removeItem(at: discoveryFile) }

  func unary(
    request: HTTPRequest<Data?>,
    onMetrics: @escaping @Sendable (HTTPMetrics) -> Void,
    onResponse: @escaping @Sendable (HTTPResponse) -> Void
  ) -> Cancelable {
    do {
      let method = request.url.lastPathComponent
      let failed = state.withLock { state in
        state.requests.append(request)
        if method == "StartStateSubscription",
          let subscription = try? Releash_Client_V1_StartStateSubscriptionRequest(
            serializedBytes: request.message ?? Data()),
          state.failedSubscriptionTargets[subscription.target, default: 0] > 0
        {
          state.failedSubscriptionTargets[subscription.target, default: 0] -= 1
          return true
        }
        return state.failedMethod == method
      }
      if failed {
        let detail = state.withLock { $0.failureDetail }
        onResponse(HTTPResponse(
          code: .internalError, headers: [:], message: nil, trailers: [:],
          error: ConnectError(code: .internalError, message: "\(method) failed", details: detail.map { [$0] } ?? []), tracingInfo: nil))
        return Cancelable(cancel: {})
      }
      if method == "CreateWorktrees", state.withLock({ $0.deferCreation }) {
        state.withLock { $0.pendingCreation = onResponse }
        return Cancelable(cancel: {})
      }
      if method == "StartStateSubscription" {
        let subscription = try Releash_Client_V1_StartStateSubscriptionRequest(
          serializedBytes: request.message ?? Data())
        let (payload, stream) = state.withLock { state in
          state.subscriptions.append(subscription)
          var payload = Releash_Client_V1_StatePayload()
          if subscription.target == "workspace-terminal" || subscription.target == "terminal" {
            payload.terminal.snapshot.replay = state.terminalReplay
            payload.terminal.snapshot.cols = 80
            payload.terminal.snapshot.rows = 24
            payload.terminal.snapshot.processedReportUnits = 5000
            payload.terminal.snapshot.status.cwd = subscription.args[0]
            payload.terminal.snapshot.status.hasForegroundProcess_p = false
          } else if subscription.target == "node-detail" {
            if let value = state.nodeDetails[subscription.args[1]] { payload.nodeDetail.value = value }
          } else if subscription.target == "agent-session" {
            if let value = state.sessions[subscription.args[0]] { payload.agentSession.value = value }
          } else if subscription.target == "session-history" {
            payload.sessionHistory = state.history
          } else if subscription.target == "workspaces" {
            payload.workspaces.repositories.items = state.repositories
          } else if subscription.target == "provider-usage" {
            payload.providerUsage = state.providerUsage
          } else if subscription.target == "provider-hook-health" {
            payload.providerHookHealth = state.hookHealth
          } else if subscription.target == "daemon-info" {
            payload.daemonInfo = state.daemonInfo
          } else if subscription.target == "repository-group-state" {
            payload.repositoryGroupState.collapsed =
              state.collapsed[subscription.args[0], default: false]
          } else if subscription.target == "workspace-state" {
            var nullable = Releash_Client_V1_NullableWorkspaceStateDto()
            if let layout = state.layouts[subscription.args[0]] { nullable.value = layout }
            payload.workspaceState = nullable
          }
          return (payload, state.streams.last)
        }
        var event = Releash_Client_V1_StateSubscriptionEvent()
        event.subscriptionID = subscription.subscriptionID
        event.snapshot = payload
        if let stream { try Self.send(event, on: stream) }
      } else if method == "SaveRepositoryGroupState" {
        let save = try Releash_Client_V1_SaveRepositoryGroupStateRequest(
          serializedBytes: request.message ?? Data())
        state.withLock { $0.collapsed[save.repositoryPath] = save.collapsed }
      } else if method == "SaveWorkspaceState" {
        let save = try Releash_Client_V1_SaveWorkspaceStateRequest(
          serializedBytes: request.message ?? Data())
        state.withLock { $0.layouts[save.worktreeName] = save.state }
        if state.withLock({ $0.deferSave }) {
          state.withLock { $0.pendingSave = onResponse }
          return Cancelable(cancel: {})
        }
      }
      var response = Data()
      if ["CreateAgentSession", "RestoreAgentSession", "ResumeAgentSessionHistoryCandidate"].contains(method) {
        var selection = Releash_Client_V1_SessionSelection()
        selection.agentSessionID = "agent-session"; selection.nodeID = "session-node"
        response = try selection.serializedData()
      } else if method == "StartWorkflow" {
        var result = Releash_Client_V1_ResultString()
        result.value = "workflow-execution"
        response = try result.serializedData()
      }
      onResponse(
        HTTPResponse(
          code: .ok, headers: ["content-type": ["application/proto"]], message: response,
          trailers: [:], error: nil, tracingInfo: .init(httpStatus: 200)))
    } catch {
      onResponse(Self.failure(error.localizedDescription))
    }
    return Cancelable(cancel: {})
  }
  func stream(request: HTTPRequest<Data?>, responseCallbacks: ResponseCallbacks)
    -> RequestCallbacks<Data>
  {
    state.withLock { state in
      state.requests.append(request)
      state.streams.append(responseCallbacks)
    }
    return RequestCallbacks(
      cancel: { Task { responseCallbacks.receiveClose(.canceled, [:], nil) } },
      sendData: { _ in
        responseCallbacks.receiveResponseHeaders(["content-type": ["application/connect+proto"]])
        var event = Releash_Client_V1_StateSubscriptionEvent()
        event.ready = .init()
        do { try Self.send(event, on: responseCallbacks) } catch {
          responseCallbacks.receiveClose(.internalError, [:], error)
        }
      }, sendClose: {})
  }
  static func failure(_ message: String) -> HTTPResponse {
    HTTPResponse(
      code: .internalError, headers: [:], message: nil, trailers: [:],
      error: ConnectError(code: .internalError, message: message), tracingInfo: nil)
  }
  static func send(_ event: Releash_Client_V1_StateSubscriptionEvent, on stream: ResponseCallbacks)
    throws
  {
    let bytes = try event.serializedData()
    let count = UInt32(bytes.count)
    var framed = Data([
      0, UInt8(truncatingIfNeeded: count >> 24), UInt8(truncatingIfNeeded: count >> 16),
      UInt8(truncatingIfNeeded: count >> 8), UInt8(truncatingIfNeeded: count),
    ])
    framed.append(bytes)
    stream.receiveResponseData(framed)
  }
  func publish(_ payload: Releash_Client_V1_StatePayload, target: String) throws {
    let (subscriptions, stream) = state.withLock { state in
      switch payload.value {
      case .workspaces(let value): state.repositories = value.repositories.items
      case .sessionHistory(let value): state.history = value
      case .agentSession(let value): if value.hasValue { state.sessions[value.value.id] = value.value }
      case .nodeDetail(let value): if value.hasValue { state.nodeDetails[value.value.id] = value.value }
      default: break
      }
      return (state.subscriptions.filter { $0.target == target }, state.streams.last)
    }
    guard let stream else { return }
    for subscription in subscriptions {
      var event = Releash_Client_V1_StateSubscriptionEvent()
      event.subscriptionID = subscription.subscriptionID; event.snapshot = payload
      try Self.send(event, on: stream)
    }
  }
  func requests(_ method: String) -> [HTTPRequest<Data?>] {
    state.withLock { $0.requests.filter { $0.url.lastPathComponent == method } }
  }
}

@MainActor func eventually(
  _ condition: () -> Bool, file: StaticString = #filePath, line: UInt = #line
) async throws {
  let deadline = ContinuousClock.now + .seconds(5)
  while !condition() && ContinuousClock.now < deadline {
    try await Task.sleep(for: .milliseconds(5))
  }
  XCTAssertTrue(condition(), file: file, line: line)
}

@MainActor func testModel(_ http: TestHTTPClient) async throws -> AppModel {
  try Data(
    #"{"port":12345,"token":"operator-test","daemon_id":"test","pid":1,"process_started_at":"1"}"#
      .utf8
  ).write(to: http.discoveryFile)
  let status = try JSONSerialization.data(withJSONObject: [
    "running": true, "compatibility": "compatible", "release_compatibility": "compatible", "discovery_file": http.discoveryFile.path,
  ])
  let model = AppModel(
    cli: CLI(
      executable: URL(fileURLWithPath: "/test/releash"),
      run: { _, args in
        XCTAssertEqual(args, ["status", "--json"])
        return status
      }), httpClient: http)
  await model.start()
  try await eventually { model.connected }
  return model
}

struct ViewElement {
  fileprivate let reference: AXUIElement
  let identifier: String
  let role: String
  let label: String
  let value: String
  let enabled: Bool
  let actions: [String]
}

@MainActor final class ViewHarness: NSObject {
  let window: NSWindow
  let existingWindows: Set<ObjectIdentifier>
  private var snapshot: [ViewElement] = []
  private var menuHandled = false
  private var trackingMenu: NSMenu?
  init<V: View>(_ view: V) async {
    existingWindows = Set(NSApp.windows.map(ObjectIdentifier.init))
    window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1000, height: 700), styleMask: [.titled], backing: .buffered, defer: false)
    super.init()
    window.isReleasedWhenClosed = false
    window.title = "ViewHarness-" + UUID().uuidString
    let host = NSHostingView(rootView: view); host.sizingOptions = []
    window.contentView = host; host.layoutSubtreeIfNeeded(); window.displayIfNeeded()
    NSApp.activate(); window.makeKeyAndOrderFront(nil)
    await refresh()
  }
  func close() { for window in NSApp.windows where !existingWindows.contains(ObjectIdentifier(window)) { window.close() } }
  func refresh() async { readSnapshot() }
  private func readSnapshot() {
    snapshot = {
      func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
        return value
      }
      var visited: [AXUIElement] = []
      func collect(_ element: AXUIElement) -> [ViewElement] {
        guard !visited.contains(where: { CFEqual($0, element) }) else { return [] }
        visited.append(element)
        var actionNames: CFArray?
        AXUIElementCopyActionNames(element, &actionNames)
        let entry = ViewElement(reference: element,
          identifier: attribute(element, kAXIdentifierAttribute) as? String ?? "",
          role: attribute(element, kAXRoleAttribute) as? String ?? "",
          label: attribute(element, kAXTitleAttribute) as? String ?? attribute(element, kAXDescriptionAttribute) as? String ?? "",
          value: attribute(element, kAXValueAttribute) as? String ?? "",
          enabled: attribute(element, kAXEnabledAttribute) as? Bool ?? false, actions: actionNames as? [String] ?? [])
        return [entry] + (attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []).flatMap(collect)
      }
      let app = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
      let windows = (attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []).filter { attribute($0, kAXTitleAttribute) as? String == window.title }
      let menus = (attribute(app, kAXChildrenAttribute) as? [AXUIElement] ?? []).filter { attribute($0, kAXRoleAttribute) as? String == kAXMenuRole }
      return (windows + menus).flatMap(collect)
    }()
  }
  func element(_ name: String) -> ViewElement? { snapshot.first { $0.identifier == name || $0.label == name || $0.value == name } }
  func value(_ element: ViewElement?) -> String? { element?.value }
  func texts() -> [String] { snapshot.flatMap { [$0.label, $0.value] } }
  func press(_ name: String, file: StaticString = #filePath, line: UInt = #line) async throws {
    await refresh()
    let element = try XCTUnwrap(snapshot.first { ($0.identifier == name || $0.label == name || $0.value == name) && $0.actions.contains(kAXPressAction) }, file: file, line: line)
    let result = AXUIElementPerformAction(element.reference, kAXPressAction as CFString).rawValue
    XCTAssertEqual(result, AXError.success.rawValue, file: file, line: line)
    await refresh()
  }
  @objc private func menuBegan(_ notification: Notification) { trackingMenu = notification.object as? NSMenu }
  func menu(_ name: String, selecting choice: String?, inspect: @escaping @MainActor ([String]) -> Void) async throws {
    await refresh()
    let row = try XCTUnwrap(element(name))
    menuHandled = false; trackingMenu = nil
    NotificationCenter.default.addObserver(self, selector: #selector(menuBegan(_:)), name: NSMenu.didBeginTrackingNotification, object: nil)
    defer { NotificationCenter.default.removeObserver(self, name: NSMenu.didBeginTrackingNotification, object: nil); trackingMenu = nil }
    let deadline = ContinuousClock.now + .seconds(5)
    await withCheckedContinuation { continuation in
      let timer = Timer(timeInterval: 0.05, repeats: true) { [self] timer in
        let handled = MainActor.assumeIsolated {
          guard let menu = trackingMenu else { return false }
          let titles = menu.items.map(\.title)
          guard let choice, let index = titles.firstIndex(of: choice) else {
            if ContinuousClock.now < deadline { return false }
            inspect(titles); XCTAssertNil(choice); menuHandled = true; menu.cancelTracking()
            return true
          }
          inspect(titles); menuHandled = true
          menu.performActionForItem(at: index); menu.cancelTracking()
          return true
        }
        if handled { timer.invalidate() }
      }
      RunLoop.main.add(timer, forMode: .eventTracking)
      DispatchQueue.main.async { [self] in
        let result = AXUIElementPerformAction(row.reference, kAXShowMenuAction as CFString)
        timer.invalidate()
        if !menuHandled { inspect([]); XCTAssertNil(choice) }
        XCTAssertEqual(result, .success)
        continuation.resume()
      }
    }
    await refresh()
  }
  func pressAlertButton(_ name: String) async throws {
    func buttons(_ view: NSView) -> [NSButton] {
      (view as? NSButton).map { [$0] } ?? view.subviews.flatMap(buttons)
    }
    let views = ([window] + window.sheets).compactMap(\.contentView)
    let button = try XCTUnwrap(views.flatMap(buttons).first { $0.title == name })
    button.performClick(nil)
    await refresh()
  }
  func setTextField(_ value: String) async throws {
    await refresh()
    let element = try XCTUnwrap(snapshot.first { $0.role == kAXTextFieldRole })
    let result = AXUIElementSetAttributeValue(element.reference, kAXValueAttribute as CFString, value as CFString).rawValue
    XCTAssertEqual(result, AXError.success.rawValue)
  }
}

@MainActor func eventuallyView(_ host: ViewHarness, _ condition: () -> Bool, file: StaticString = #filePath, line: UInt = #line) async throws {
  let deadline = ContinuousClock.now + .seconds(5)
  repeat {
    await host.refresh()
    if condition() { return }
    try await Task.sleep(for: .milliseconds(20))
  } while ContinuousClock.now < deadline
  XCTAssertTrue(condition(), host.texts().joined(separator: " | "), file: file, line: line)
}

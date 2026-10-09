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
    var failedMethod: String?
    var pendingCreation: (@Sendable (HTTPResponse) -> Void)?
    var deferCreation = false
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
        return state.failedMethod == method
      }
      if failed {
        onResponse(Self.failure("\(method) failed"))
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
          if subscription.target == "workspaces" {
            payload.workspaces.repositories.items = state.repositories
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
      } else if method == "SaveWorkspaceState" {
        let save = try Releash_Client_V1_SaveWorkspaceStateRequest(
          serializedBytes: request.message ?? Data())
        state.withLock { $0.layouts[save.worktreeName] = save.state }
      }
      onResponse(
        HTTPResponse(
          code: .ok, headers: ["content-type": ["application/proto"]], message: Data(),
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
      cancel: { responseCallbacks.receiveClose(.canceled, [:], nil) },
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
    "running": true, "compatibility": "compatible", "discovery_file": http.discoveryFile.path,
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

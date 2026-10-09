import AppKit
import Connect
import Foundation
import Observation
import SwiftProtobuf

@MainActor @Observable final class AppModel {
  var repositories: [Releash_Client_V1_WorkspaceRepositoryList] = []
  var collapsed: [String: Bool] = [:]
  var selectedWorktree: String?
  var layout: PaneLayout = .empty()
  var failure: String?
  var connectionFailure: String?
  var connected = false
  var reconnecting = false
  var layoutLoaded = false
  var workflows: [Releash_Client_V1_WorkflowSummaryDto] = []
  var providers: [Releash_Client_V1_AgentSessionProviderDto] = []
  @ObservationIgnored private var workspaceState = Releash_Client_V1_WorkspaceStateDto()
  @ObservationIgnored private var cli: CLI?
  @ObservationIgnored private var policy: TransportPolicy?
  @ObservationIgnored private var streamTask: Task<Void, Never>?
  @ObservationIgnored private var heartbeatTask: Task<Void, Never>?
  @ObservationIgnored private var callbacks: [String: (Releash_Client_V1_StatePayload) -> Void] =
    [:]
  @ObservationIgnored private var targets: [String: (String, [String])] = [:]
  @ObservationIgnored private var clientID: String?
  @ObservationIgnored private var subscriptionIDs = StateSubscriptionIDs()
  @ObservationIgnored private var host = ""
  @ObservationIgnored private(set) var client: (any Releash_Client_V1_ClientServiceClientInterface)?
  @ObservationIgnored private(set) var headers: Headers = [:]
  @ObservationIgnored var pendingSaves: [String: Int] = [:]
  @ObservationIgnored private var saves: [String: Task<Void, Never>] = [:]

  @ObservationIgnored private let httpClient: any HTTPClientInterface

  init(cli: CLI? = nil, httpClient: any HTTPClientInterface = URLSessionHTTPClient()) {
    self.cli = cli
    self.httpClient = httpClient
  }

  func stop() async {
    streamTask?.cancel()
    heartbeatTask?.cancel()
    await streamTask?.value
  }

  func start() async {
    streamTask?.cancel()
    await streamTask?.value
    do {
      if cli == nil {
        let executable = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/releash")
        guard FileManager.default.isExecutableFile(atPath: executable.path) else {
          throw CLIFailure(message: "Bundled releash CLI is missing")
        }
        cli = CLI(executable: executable)
      }
      policy = try TransportPolicy()
      try await connect(startIfMissing: true)
      connectionFailure = nil
      subscribe("workspaces") { [weak self] payload in
        guard case .workspaces(let value) = payload.value else { return }
        self?.repositories = value.repositories.items
        for repo in value.repositories.items {
          self?.subscribe("repository-group-state", [repo.path]) { [weak self] payload in
            if case .repositoryGroupState(let group) = payload.value {
              self?.collapsed[repo.path] = group.collapsed
            }
          }
        }
      }
      subscribe("workflows") { [weak self] payload in
        if case .workflows(let list) = payload.value { self?.workflows = list.items }
      }
      subscribe("providers") { [weak self] payload in
        if case .providers(let list) = payload.value { self?.providers = list.items }
      }
      streamTask = Task { await recoverStream() }
    } catch { connectionFailure = failureMessage(error) }
  }
  private func connect(startIfMissing: Bool) async throws {
    guard let cli, let policy else { throw CLIFailure(message: "Connection is not initialized") }
    let discovery = try await cli.discover(startIfMissing: startIfMissing)
    host = "http://127.0.0.1:\(discovery.port)"
    headers = ["authorization": ["Bearer \(discovery.token)"]]
    client = Releash_Client_V1_ClientServiceClient(
      client: ProtocolClient(
        httpClient: httpClient,
        config: ProtocolClientConfig(host: host, codec: ProtoCodec(), timeout: policy.timeout)))
  }
  func subscribe(
    _ target: String, _ args: [String] = [],
    onValue: @escaping (Releash_Client_V1_StatePayload) -> Void
  ) {
    let key = ([target] + args).map { "\($0.utf8.count):\($0)" }.joined()
    callbacks[key] = onValue
    guard targets[key] == nil else { return }
    targets[key] = (target, args)
    if let clientID { perform { try await self.register(key, clientID: clientID) } }
  }
  func unsubscribe(_ target: String, _ args: [String] = []) {
    let key = ([target] + args).map { "\($0.utf8.count):\($0)" }.joined()
    targets.removeValue(forKey: key)
    callbacks.removeValue(forKey: key)
    guard let id = subscriptionIDs.remove(key), let client, clientID != nil else { return }
    var request = Releash_Client_V1_StopStateSubscriptionRequest()
    request.subscriptionID = id
    perform {
      _ = try await client.stopStateSubscription(request: request, headers: self.headers).result
        .get()
    }
  }
  private func register(_ key: String, clientID: String) async throws {
    guard let client, let (target, args) = targets[key] else { return }
    let reservation = subscriptionIDs.reserve(key)
    if let previous = reservation.previous {
      var stop = Releash_Client_V1_StopStateSubscriptionRequest()
      stop.subscriptionID = previous
      _ = try await client.stopStateSubscription(request: stop, headers: headers).result.get()
    }
    guard self.clientID == clientID, targets[key] != nil,
      subscriptionIDs.key(for: reservation.id) == key
    else { return }
    var request = Releash_Client_V1_StartStateSubscriptionRequest()
    request.clientID = clientID
    request.target = target
    request.args = args
    request.subscriptionID = reservation.id
    _ = try await client.startStateSubscription(request: request, headers: headers).result.get()
  }
  private func recoverStream() async {
    guard let policy else { return }
    var attempt = 0
    while !Task.isCancelled {
      do {
        if attempt > 0 { try await connect(startIfMissing: false) }
        guard client != nil else { return }
        let id = UUID().uuidString
        let streamingClient = Releash_Client_V1_ClientServiceClient(
          client: ProtocolClient(
            httpClient: httpClient,
            config: ProtocolClientConfig(host: host, codec: ProtoCodec())))
        let stream = streamingClient.openStateStream(headers: headers)
        defer { stream.cancel() }
        var request = Releash_Client_V1_OpenStateStreamRequest()
        request.clientID = id
        try stream.send(request)
        let began = Date()
        func alive() {
          heartbeatTask?.cancel()
          heartbeatTask = Task {
            do {
              try await Task.sleep(for: .seconds(policy.silence))
              stream.cancel()
            } catch {}
          }
        }
        alive()
        for await result in stream.results() {
          if Task.isCancelled {
            stream.cancel()
            break
          }
          alive()
          switch result {
          case .message(let event):
            if case .ready = event.event, event.subscriptionID.isEmpty {
              clientID = id
              subscriptionIDs.reset()
              connected = true
              reconnecting = false
              for key in targets.keys.sorted() { try await register(key, clientID: id) }
            } else if case .snapshot(let payload) = event.event {
              if let key = subscriptionIDs.key(for: event.subscriptionID) {
                callbacks[key]?(payload)
              }
            } else if case .change(let change) = event.event {
              if let key = subscriptionIDs.key(for: event.subscriptionID) {
                callbacks[key]?(change.payload)
              }
            } else if case .failure(let error) = event.event {
              failure = error.message
            }
          case .complete(_, let error, _): if let error { throw error }
          case .headers: break
          }
        }
        if Date().timeIntervalSince(began) * 1000
          >= Double(policy.options.Releash_Client_V1_connectionBackoff.resetAfterMs)
        {
          attempt = 0
        }
      } catch { failure = failureMessage(error) }
      heartbeatTask?.cancel()
      clientID = nil
      subscriptionIDs.reset()
      reconnecting = true
      do { try await Task.sleep(for: .seconds(policy.backoff(attempt: attempt))) } catch { return }
      attempt += 1
    }
  }
  private func failureMessage(_ error: Error) -> String {
    if let error = error as? ConnectError, let message = error.message { return message }
    return error.localizedDescription
  }
  func perform(_ operation: @escaping @MainActor () async throws -> Void) {
    Task { do { try await operation() } catch { failure = failureMessage(error) } }
  }
  func selectWorktree(_ path: String) {
    if let selectedWorktree {
      unsubscribe("workspace-state", [selectedWorktree, selectedWorktree])
    }
    selectedWorktree = path
    layoutLoaded = false
    layout = .empty()
    subscribe("workspace-state", [path, path]) { [weak self] payload in
      guard let self, case .workspaceState(let value) = payload.value, selectedWorktree == path
      else { return }
      receiveWorkspaceState(value, path: path)
    }
  }
  func receiveWorkspaceState(_ value: Releash_Client_V1_NullableWorkspaceStateDto, path: String) {
    guard selectedWorktree == path, pendingSaves[path, default: 0] == 0 else { return }
    layoutLoaded = true
    if value.hasValue {
      workspaceState = value.value
      if value.value.hasPaneLayoutJson {
        do {
          layout = try JSONDecoder().decode(
            PaneLayout.self, from: Data(value.value.paneLayoutJson.utf8))
        } catch { failure = failureMessage(error) }
      }
    } else {
      workspaceState = Releash_Client_V1_WorkspaceStateDto()
      workspaceState.version = 1
      workspaceState.tabs.editors = Releash_Client_V1_ListWorkspaceTabEntryDto()
      workspaceState.layout.centerTab.value = .agent
      workspaceState.layout.activeView = "session"
      workspaceState.layout.leftNavCollapsed = false
      workspaceState.layout.rightCollapsed = true
      workspaceState.layout.rightBottomCollapsed = true
    }
  }
  func editLayout(_ edit: (inout PaneLayout) -> Void) {
    guard layoutLoaded, let path = selectedWorktree, let client else { return }
    edit(&layout)
    do {
      var state = workspaceState
      state.paneLayoutJson = String(decoding: try JSONEncoder().encode(layout), as: UTF8.self)
      workspaceState = state
      var request = Releash_Client_V1_SaveWorkspaceStateRequest()
      request.worktreeName = path
      request.state = state
      let prior = saves[path]
      pendingSaves[path, default: 0] += 1
      saves[path] = Task {
        await prior?.value
        do {
          _ = try await client.saveWorkspaceState(request: request, headers: headers).result.get()
        } catch { failure = failureMessage(error) }
        pendingSaves[path, default: 0] -= 1
        if pendingSaves[path] == 0 {
          pendingSaves.removeValue(forKey: path)
          saves.removeValue(forKey: path)
          if let clientID {
            let key = (["workspace-state", path, path]).map { "\($0.utf8.count):\($0)" }.joined()
            do { try await register(key, clientID: clientID) } catch {
              failure = failureMessage(error)
            }
          }
        }
      }
    } catch { failure = failureMessage(error) }
  }
  func setCollapsed(_ path: String, _ value: Bool) {
    guard let client else { return }
    var request = Releash_Client_V1_SaveRepositoryGroupStateRequest()
    request.repositoryPath = path
    request.collapsed = value
    perform {
      _ = try await client.saveRepositoryGroupState(request: request, headers: self.headers).result
        .get()
    }
  }
  func refresh() {
    guard let client else { return }
    perform {
      _ = try await client.refreshWorkspaces(request: .init(), headers: self.headers).result.get()
    }
  }
  func addRepository() {
    let panel = NSOpenPanel()
    panel.canChooseDirectories = true
    panel.canChooseFiles = false
    guard panel.runModal() == .OK, let path = panel.url?.path else { return }
    addRepository(path)
  }
  func addRepository(_ path: String) {
    guard let client else { return }
    var request = Releash_Client_V1_AddRepoPathRequest()
    request.path = path
    perform {
      _ = try await client.addRepoPath(request: request, headers: self.headers).result.get()
    }
  }
}

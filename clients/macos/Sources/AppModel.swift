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
  var panes = PaneWorkspace()
  var failure: String?
  var connectionFailure: String?
  var connected = false
  var reconnecting = false
  var layoutLoaded = false
  var workflows: [Releash_Client_V1_WorkflowSummaryDto] = []
  var providers: [Releash_Client_V1_AgentSessionProviderDto] = []
  @ObservationIgnored var confirmTerminalClose: () -> Bool = {
    let alert = NSAlert()
    alert.messageText = "実行中の処理を停止してTerminalを閉じますか？"
    alert.addButton(withTitle: "閉じる")
    alert.addButton(withTitle: "キャンセル")
    return alert.runModal() == .alertFirstButtonReturn
  }
  @ObservationIgnored private var terminals: [String: TerminalSession] = [:]
  @ObservationIgnored private var workspaceState = Releash_Client_V1_WorkspaceStateDto()
  @ObservationIgnored private var cli: CLI?
  @ObservationIgnored private var policy: TransportPolicy?
  @ObservationIgnored private var streamTask: Task<Void, Never>?
  @ObservationIgnored private var heartbeatTask: Task<Void, Never>?
  @ObservationIgnored private var callbacks: [String: [UUID: (Releash_Client_V1_StatePayload) -> Void]] =
    [:]
  @ObservationIgnored private var subscriberKeys: [UUID: String] = [:]
  @ObservationIgnored private var lastPayloads: [String: Releash_Client_V1_StatePayload] = [:]
  @ObservationIgnored private var groupSubscriptions: [String: UUID] = [:]
  @ObservationIgnored private var rootSubscriptions: [UUID] = []
  @ObservationIgnored private var workspaceSubscription: UUID?
  @ObservationIgnored private var desiredTargets: [String: (String, [String])] = [:]
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
      for id in rootSubscriptions { unsubscribe(id) }
      rootSubscriptions = []
      rootSubscriptions.append(subscribe("workspaces") { [weak self] payload in
        guard case .workspaces(let value) = payload.value else { return }
        guard let self else { return }
        let paths = Set(value.repositories.items.map(\.path))
        for repo in repositories where !paths.contains(repo.path) {
          if let id = groupSubscriptions.removeValue(forKey: repo.path) { unsubscribe(id) }
          collapsed.removeValue(forKey: repo.path)
        }
        repositories = value.repositories.items
        for repo in value.repositories.items where groupSubscriptions[repo.path] == nil {
          groupSubscriptions[repo.path] = self.subscribe("repository-group-state", [repo.path]) { [weak self] payload in
            if case .repositoryGroupState(let group) = payload.value {
              self?.collapsed[repo.path] = group.collapsed
            }
          }
        }
      })
      rootSubscriptions.append(subscribe("workflows") { [weak self] payload in
        if case .workflows(let list) = payload.value { self?.workflows = list.items }
      })
      rootSubscriptions.append(subscribe("providers") { [weak self] payload in
        if case .providers(let list) = payload.value { self?.providers = list.items }
      })
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
  private func subscriptionKey(_ target: String, _ args: [String]) -> String {
    ([target] + args).map { "\($0.utf8.count):\($0)" }.joined()
  }
  @discardableResult func subscribe(
    _ target: String, _ args: [String] = [],
    onValue: @escaping (Releash_Client_V1_StatePayload) -> Void
  ) -> UUID {
    let key = subscriptionKey(target, args)
    let id = UUID()
    subscriberKeys[id] = key
    callbacks[key, default: [:]][id] = onValue
    desiredTargets[key] = (target, args)
    if let payload = lastPayloads[key] { onValue(payload) }
    if targets[key] == nil {
      targets[key] = (target, args)
      if let clientID { perform { try await self.register(key, clientID: clientID) } }
    }
    return id
  }
  func unsubscribe(_ subscriber: UUID) {
    guard let key = subscriberKeys.removeValue(forKey: subscriber) else { return }
    callbacks[key]?.removeValue(forKey: subscriber)
    guard callbacks[key]?.isEmpty == true else { return }
    desiredTargets.removeValue(forKey: key)
    targets.removeValue(forKey: key)
    callbacks.removeValue(forKey: key)
    lastPayloads.removeValue(forKey: key)
    guard let id = subscriptionIDs.remove(key), let client, clientID != nil else { return }
    var request = Releash_Client_V1_StopStateSubscriptionRequest()
    request.subscriptionID = id
    perform {
      _ = try await client.stopStateSubscription(request: request, headers: self.headers).result
        .get()
    }
  }
  private func receive(_ payload: Releash_Client_V1_StatePayload, key: String) {
    if case .terminal = payload.value {} else { lastPayloads[key] = payload }
    for callback in callbacks[key]?.values ?? [:].values { callback(payload) }
  }
  private func register(_ key: String, clientID: String) async throws {
    guard let client, let (target, args) = targets[key] else { return }
    if target == "workspace-terminal", args.count == 2 {
      terminals[key]?.disconnect()
      var spawn = Releash_Client_V1_GetOrSpawnTerminalSurfaceRequest()
      spawn.owner.workspaceTerminal.workspacePath = args[0]
      spawn.owner.workspaceTerminal.terminalID = args[1]
      spawn.cwd = args[0]
      spawn.rows = UIStyle.terminalRows
      spawn.cols = UIStyle.terminalColumns
      _ = try await client.getOrSpawnTerminalSurface(request: spawn, headers: headers).result.get()
    }
    let reservation = subscriptionIDs.reserve(key)
    do {
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
    } catch {
      if subscriptionIDs.key(for: reservation.id) == key {
        targets.removeValue(forKey: key)
        _ = subscriptionIDs.remove(key)
      }
      throw error
    }
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
              targets = desiredTargets
              connected = true
              reconnecting = false
              for key in targets.keys.sorted() { try await register(key, clientID: id) }
            } else if case .snapshot(let payload) = event.event {
              if let key = subscriptionIDs.key(for: event.subscriptionID) {
                receive(payload, key: key)
              }
            } else if case .change(let change) = event.event {
              if let key = subscriptionIDs.key(for: event.subscriptionID) {
                receive(change.payload, key: key)
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
      for terminal in terminals.values { terminal.disconnect() }
      subscriptionIDs.reset()
      reconnecting = true
      do { try await Task.sleep(for: .seconds(policy.backoff(attempt: attempt))) } catch { return }
      attempt += 1
    }
  }
  func failureMessage(_ error: Error) -> String {
    if let error = error as? ConnectError {
      let detail: Releash_Client_V1_CommandError? = error.unpackedDetails().first
      switch detail?.variant {
      case .message(let message) where !message.value.isEmpty: return message.value
      case .coded(let coded): return coded.message.isEmpty ? coded.code : coded.message
      default: break
      }
      if let message = error.message { return message }
    }
    return error.localizedDescription
  }
  func perform(_ operation: @escaping @MainActor () async throws -> Void) {
    Task { do { try await operation() } catch { failure = failureMessage(error) } }
  }
  func selectWorktree(_ path: String) {
    guard selectedWorktree != path || !layoutLoaded else { return }
    if let id = workspaceSubscription { unsubscribe(id) }
    stopTerminals()
    selectedWorktree = path
    layoutLoaded = false
    layout = .empty()

    workspaceSubscription = subscribe("workspace-state", [path, path]) { [weak self] payload in
      guard let self, case .workspaceState(let value) = payload.value, selectedWorktree == path
      else { return }
      receiveWorkspaceState(value, path: path)
    }
  }
  func deselectWorktree(_ path: String) {
    guard selectedWorktree == path else { return }
    if let id = workspaceSubscription { unsubscribe(id) }
    workspaceSubscription = nil
    stopTerminals()
    selectedWorktree = nil
    layoutLoaded = false
    layout = .empty()

  }
  func receiveWorkspaceState(_ value: Releash_Client_V1_NullableWorkspaceStateDto, path: String) {
    guard selectedWorktree == path, pendingSaves[path, default: 0] == 0 else { return }
    if value.hasValue {
      workspaceState = value.value
      if value.value.hasPaneLayout {
        do {
          layout = try PaneLayout(value.value.paneLayout)
        } catch { failure = failureMessage(error) }
      } else {
        layout = .empty()

      }
    } else {
      layout = .empty()

      workspaceState = Releash_Client_V1_WorkspaceStateDto()
      workspaceState.version = 1
      workspaceState.tabs.editors = Releash_Client_V1_ListWorkspaceTabEntryDto()
      workspaceState.layout.centerTab.value = .agent
      workspaceState.layout.activeView = "session"
      workspaceState.layout.leftNavCollapsed = false
      workspaceState.layout.rightCollapsed = true
      workspaceState.layout.rightBottomCollapsed = true
    }
    do {
      if (try? panes.layout()) != layout { try panes.restore(layout) }
    } catch {
      failure = failureMessage(error)
      return
    }
    layoutLoaded = true
    syncTerminalTabs(layout)
  }
  func editLayout(_ edit: (inout PaneLayout) -> Void) {
    guard layoutLoaded, let path = selectedWorktree, let client else { return }
    let previous = layout
    edit(&layout)
    guard layout != previous else { return }
    var state = workspaceState
    state.paneLayout = layout.message
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
          let key = subscriptionKey("workspace-state", [path, path])
          do { try await register(key, clientID: clientID) } catch {
            failure = failureMessage(error)
          }
        }
      }
    }
  }
  func terminal(path: String, id: String) -> TerminalSession {
    let key = subscriptionKey("workspace-terminal", [path, id])
    if let session = terminals[key] { return session }
    let session = TerminalSession(model: self, path: path, id: id)
    terminals[key] = session
    return session
  }

  func syncTerminalTabs(_ layout: PaneLayout) {
    guard let path = selectedWorktree else { return }
    let ids = Set(layout.tabs.filter { $0.kind == .terminal }.map(\.id))
    for (key, session) in terminals where !ids.contains(session.id) { session.stop(); terminals.removeValue(forKey: key) }
    for id in ids { _ = terminal(path: path, id: id) }
  }

  private func stopTerminals() {
    for session in terminals.values { session.stop() }
    terminals.removeAll()
  }

  func terminalAttachment(path: String, id: String) -> String? {
    subscriptionIDs.id(for: subscriptionKey("workspace-terminal", [path, id]))
  }

  func restartTerminalSubscription(path: String, id: String) {
    guard let clientID else { return }
    let key = subscriptionKey("workspace-terminal", [path, id])
    perform { try await self.register(key, clientID: clientID) }
  }

  func closeTerminal(_ tab: PaneTab) async -> Bool {
    guard let path = selectedWorktree, let client else { return false }
    let session = terminal(path: path, id: tab.id)
    guard session.exited || session.hasForegroundProcess != nil else {
      failure = "Terminalの状態を確認できません。接続の回復後に閉じてください。"
      return false
    }
    if session.hasForegroundProcess == true && !confirmTerminalClose() { return false }
    do {
      var request = Releash_Client_V1_KillTerminalSurfaceRequest()
      request.owner = session.owner
      _ = try await client.killTerminalSurface(request: request, headers: headers).result.get()
      session.stop()
      return true
    } catch { failure = failureMessage(error); return false }
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

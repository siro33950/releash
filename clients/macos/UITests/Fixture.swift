import Connect
import Foundation
import XCTest
#if canImport(Releash)
import Releash
#endif

final class Fixture: Sendable {
  let root: URL
  let directory: URL
  let data: URL
  let cli: URL
  let providerExecutable: String
  let environment: [String: String]

  init(providerUsage: Bool = false) throws {
    guard let path = ProcessInfo.processInfo.environment["RELEASH_E2E_ROOT"] else {
      throw NSError(domain: "E2E", code: 1, userInfo: [NSLocalizedDescriptionKey: "Missing RELEASH_E2E_ROOT"])
    }
    root = URL(fileURLWithPath: path).standardizedFileURL
    directory = FileManager.default.temporaryDirectory.appendingPathComponent("releash-ui-\(UUID())")
    data = directory.appendingPathComponent("data")
    cli = root.appendingPathComponent("clients/macos/.build/Helpers/Debug/releash")
    let manifest = try JSONDecoder().decode([String: String].self, from: Data(contentsOf:
      root.appendingPathComponent("clients/macos/.build/agent-tui-fixture.json")))
    guard let executable = manifest["executable"] else {
      throw NSError(domain: "E2E", code: 2, userInfo: [NSLocalizedDescriptionKey: "Missing TUI fixture executable path"])
    }
    providerExecutable = URL(fileURLWithPath: executable).path
    let fixture = URL(fileURLWithPath: providerExecutable)
    guard FileManager.default.isExecutableFile(atPath: fixture.path) else {
      throw NSError(domain: "E2E", code: 2, userInfo: [NSLocalizedDescriptionKey: "Missing TUI fixture"])
    }
    let plan: [String: Any] = [
      "label": "e2e-provider", "input_lines": 1000, "alternate_screen": true,
      "report_terminal_size": false, "lifecycle_endpoint": "", "lifecycle": [], "exit_code": 0,
    ]
    var launchEnvironment = [
      "RELEASH_DATA_DIR": data.path,
      "RELEASH_WORKFLOWS_DIR": data.appendingPathComponent("workflows").path,
      "RELEASH_AGENT_TUI_FIXTURE_PLAN": String(decoding: try JSONSerialization.data(withJSONObject: plan), as: UTF8.self),
    ]
    if providerUsage {
      launchEnvironment["CLAUDE_CONFIG_DIR"] = directory.appendingPathComponent("claude").path
      launchEnvironment["RELEASH_AGENT_TUI_FIXTURE_STATUSLINE"] =
        #"{"rate_limits":{"five_hour":{"used_percentage":81,"resets_at":4102444800},"seven_day":{"used_percentage":41,"resets_at":4102444800}}}"#
    }
    environment = launchEnvironment
    try FileManager.default.createDirectory(at: data, withIntermediateDirectories: true)
    let encoder = JSONEncoder()
    encoder.outputFormatting = .withoutEscapingSlashes
    let quoted = String(decoding: try encoder.encode(providerExecutable), as: UTF8.self)
    try "[agents.claude]\ncli_path = \(quoted)\n[agents.codex]\ncli_path = \(quoted)\n".write(
      to: data.appendingPathComponent("releash.toml"), atomically: true, encoding: .utf8)
  }

  @discardableResult func process(_ executable: URL, _ arguments: [String], at path: URL? = nil) throws -> Data {
    let process = Process()
    process.executableURL = executable
    process.arguments = arguments
    process.currentDirectoryURL = path ?? directory
    process.environment = ProcessInfo.processInfo.environment.merging(environment) { _, new in new }
    let output = Pipe()
    process.standardOutput = output
    process.standardError = output
    try process.run()
    let bytes = output.fileHandleForReading.readDataToEndOfFile()
    process.waitUntilExit()
    guard process.terminationStatus == 0 else {
      throw NSError(domain: "E2E", code: Int(process.terminationStatus), userInfo: [
        NSLocalizedDescriptionKey: "\(arguments): \(String(decoding: bytes, as: UTF8.self))"
      ])
    }
    return bytes
  }

  func running() throws -> Bool {
    let value = try JSONSerialization.jsonObject(with: process(cli, ["status", "--json"])) as! [String: Any]
    return value["running"] as! Bool
  }

  func client() throws -> (Releash_Client_V1_ClientServiceClient, Headers) {
    let value = try JSONSerialization.jsonObject(with: Data(contentsOf: data.appendingPathComponent("client-api.json"))) as! [String: Any]
    let port = value["port"] as! Int
    let token = value["token"] as! String
    return (Releash_Client_V1_ClientServiceClient(client: ProtocolClient(
      httpClient: URLSessionHTTPClient(), config: ProtocolClientConfig(host: "http://127.0.0.1:\(port)", codec: ProtoCodec()))),
      ["authorization": ["Bearer \(token)"]])
  }

  func startServer() async throws {
    try process(cli, ["server", "start"])
    try await waitForState(target: "provider-availability") { payload in
      guard case .providerAvailability(let availability) = payload.value else { return false }
      try Self.verifyProviders(availability, executable: self.providerExecutable)
      return true
    }
  }

  static func verifyProviders(_ availability: Releash_Client_V1_ProviderAvailabilitySnapshotResponse,
    executable: String) throws
  {
    for provider in ["claude", "codex"] {
      guard let item = availability.providers.items.first(where: { $0.provider == provider }),
        item.hasResolvedExecutable, item.resolvedExecutable == executable
      else {
        throw NSError(domain: "E2E", code: 4, userInfo: [
          NSLocalizedDescriptionKey: "Provider \(provider) did not resolve to \(executable)"
        ])
      }
    }
  }

  func repository(_ name: String, worktrees: [String] = [], workflows: [String] = [], threads: [String] = []) async throws -> URL {
    guard try running() else {
      throw NSError(domain: "E2E", code: 4, userInfo: [NSLocalizedDescriptionKey: "Start the server before preparing a repository"])
    }
    let repository = directory.appendingPathComponent(name)
    try FileManager.default.createDirectory(at: repository, withIntermediateDirectories: true)
    let git = URL(fileURLWithPath: "/usr/bin/git")
    try process(git, ["init", "-b", "main"], at: repository)
    try process(git, ["-c", "user.name=E2E", "-c", "user.email=e2e@example.invalid", "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-m", "initial"], at: repository)
    for branch in worktrees {
      try process(git, ["worktree", "add", "-b", branch, directory.appendingPathComponent("\(name)-\(branch)").path], at: repository)
    }
    let (client, headers) = try client()
    var add = Releash_Client_V1_AddRepoPathRequest()
    add.path = repository.path
    _ = try await client.addRepoPath(request: add, headers: headers).result.get()
    for name in workflows {
      var request = Releash_Client_V1_StartWorkflowRequest()
      request.workflowName = name
      request.worktreePath = repository.path
      let execution = try await client.startWorkflow(request: request, headers: headers).result.get()
      try await waitForState(target: "workspaces") { payload in
        guard case .workspaces(let workspaces) = payload.value else { return false }
        return Self.hasRunningWorkflow(workspaces, repository: repository.path, executionID: execution.value)
      }
    }
    for content in threads {
      var request = Releash_Client_V1_CreateReviewThreadRequest()
      request.worktreeName = repository.path
      request.content = content
      _ = try await client.createReviewThread(request: request, headers: headers).result.get()
    }
    return repository
  }

  func waitForLiveSession(repository: URL, branch: String,
    provider: Releash_Client_V1_AgentSessionProviderDto.Value) async throws
  {
    try await waitForState(target: "workspaces") { payload in
      guard case .workspaces(let workspaces) = payload.value,
          let repo = workspaces.repositories.items.first(where: { $0.path == repository.path }),
          let created = repo.branches.items.first(where: { $0.name == branch }),
          let worktree = repo.worktrees.items.first(where: { $0.path == created.worktreePath }),
          worktree.executions.count == 1,
          let execution = worktree.executions.first,
          !execution.isWorkflow, execution.provider.value == provider,
          worktree.snapshot.nodes.items.count == 1,
          case .node(let node) = worktree.snapshot.nodes.items.first?.variant,
          node.processPresence.value == .live
        else { return false }
      return true
    }
  }

  static func hasRunningWorkflow(_ workspaces: Releash_Client_V1_WorkspaceListSnapshot,
    repository: String, executionID: String) -> Bool
  {
    guard let repo = workspaces.repositories.items.first(where: { $0.path == repository }),
      let main = repo.branches.items.first(where: { $0.isMainWorktree }),
      let worktree = repo.worktrees.items.first(where: { $0.path == main.worktreePath })
    else { return false }
    return worktree.executions.contains { $0.id == executionID && $0.isWorkflow && $0.status == "active" }
  }

  func waitForState(target: String, args: [String] = [], isolation: isolated (any Actor)? = #isolation,
    matches: (Releash_Client_V1_StatePayload) throws -> Bool) async throws
  {
    let (client, headers) = try client()
    let stream = client.openStateStream(headers: headers)
    let timeout = DispatchWorkItem { stream.cancel() }
    DispatchQueue.global().asyncAfter(deadline: .now() + 30, execute: timeout)
    defer {
      timeout.cancel()
      stream.cancel()
    }
    let clientID = UUID().uuidString
    let subscriptionID = UUID().uuidString
    var open = Releash_Client_V1_OpenStateStreamRequest()
    open.clientID = clientID
    try stream.send(open)
    for await result in stream.results() {
      switch result {
      case .message(let event):
        if case .ready = event.event, event.subscriptionID.isEmpty {
          var subscription = Releash_Client_V1_StartStateSubscriptionRequest()
          subscription.clientID = clientID
          subscription.subscriptionID = subscriptionID
          subscription.target = target
          subscription.args = args
          _ = try await client.startStateSubscription(request: subscription, headers: headers).result.get()
        }
        let payload: Releash_Client_V1_StatePayload
        switch event.event {
        case .snapshot(let value): payload = value
        case .change(let value): payload = value.payload
        case .failure(let error):
          throw NSError(domain: "E2E", code: 4, userInfo: [NSLocalizedDescriptionKey: error.message])
        default: continue
        }
        if event.subscriptionID == subscriptionID, try matches(payload) { return }
      case .complete(_, let error, _):
        if let error { throw error }
      case .headers: break
      }
    }
    throw NSError(domain: "E2E", code: 4, userInfo: [
      NSLocalizedDescriptionKey: "Expected state was not observed for \(target) within 30 seconds"
    ])
  }

  func cleanUp() throws {
    try Self.cleanUp(stop: {
      if try running() { try process(cli, ["server", "stop"]) }
    }, remove: {
      try FileManager.default.removeItem(at: directory)
    })
  }

  static func cleanUp(stop: () throws -> Void, remove: () throws -> Void) throws {
    let stopped = Result { try stop() }
    let removed = Result { try remove() }
    try stopped.get()
    try removed.get()
  }
}

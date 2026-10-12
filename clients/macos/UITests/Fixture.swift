import Connect
import Foundation
import XCTest

final class Fixture: Sendable {
  let root: URL
  let directory: URL
  let data: URL
  let cli: URL
  let environment: [String: String]

  init() throws {
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
    let fixture = URL(fileURLWithPath: executable)
    guard FileManager.default.isExecutableFile(atPath: fixture.path) else {
      throw NSError(domain: "E2E", code: 2, userInfo: [NSLocalizedDescriptionKey: "Missing TUI fixture"])
    }
    let plan: [String: Any] = [
      "label": "e2e-provider", "input_lines": 1000, "alternate_screen": true,
      "report_terminal_size": false, "lifecycle_endpoint": "", "lifecycle": [], "exit_code": 0,
    ]
    environment = [
      "RELEASH_DATA_DIR": data.path,
      "RELEASH_AGENT_TUI_FIXTURE_PLAN": String(decoding: try JSONSerialization.data(withJSONObject: plan), as: UTF8.self),
    ]
    try FileManager.default.createDirectory(at: data, withIntermediateDirectories: true)
    let encoder = JSONEncoder()
    encoder.outputFormatting = .withoutEscapingSlashes
    let quoted = String(decoding: try encoder.encode(fixture.path), as: UTF8.self)
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

  func repository(_ name: String, worktrees: [String] = [], workflows: [String] = [], threads: [String] = []) async throws -> URL {
    let repository = directory.appendingPathComponent(name)
    try FileManager.default.createDirectory(at: repository, withIntermediateDirectories: true)
    let git = URL(fileURLWithPath: "/usr/bin/git")
    try process(git, ["init", "-b", "main"], at: repository)
    try process(git, ["-c", "user.name=E2E", "-c", "user.email=e2e@example.invalid", "commit", "--allow-empty", "-m", "initial"], at: repository)
    for branch in worktrees {
      try process(git, ["worktree", "add", "-b", branch, directory.appendingPathComponent("\(name)-\(branch)").path], at: repository)
    }
    try process(cli, ["server", "start"])
    let (client, headers) = try client()
    var add = Releash_Client_V1_AddRepoPathRequest()
    add.path = repository.path
    _ = try await client.addRepoPath(request: add, headers: headers).result.get()
    for name in workflows {
      var request = Releash_Client_V1_StartWorkflowRequest()
      request.workflowName = name
      request.worktreePath = repository.path
      _ = try await client.startWorkflow(request: request, headers: headers).result.get()
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
    let (client, headers) = try client()
    let stream = client.openStateStream(headers: headers)
    let timeout = Task {
      try await Task.sleep(for: .seconds(30))
      stream.cancel()
    }
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
          subscription.target = "workspaces"
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
        guard event.subscriptionID == subscriptionID,
          case .workspaces(let workspaces) = payload.value,
          let repo = workspaces.repositories.items.first(where: { $0.path == repository.path }),
          let created = repo.branches.items.first(where: { $0.name == branch }),
          let worktree = repo.worktrees.items.first(where: { $0.path == created.worktreePath }),
          worktree.executions.count == 1,
          let execution = worktree.executions.first,
          !execution.isWorkflow, execution.provider.value == provider,
          worktree.snapshot.nodes.items.count == 1,
          case .node(let node) = worktree.snapshot.nodes.items.first?.variant,
          node.processPresence.value == .live
        else { continue }
        return
      case .complete(_, let error, _):
        if let error { throw error }
      case .headers: break
      }
    }
    throw NSError(domain: "E2E", code: 4, userInfo: [
      NSLocalizedDescriptionKey: "No live \(provider) Session observed in \(repository.path) / \(branch)"
    ])
  }

  func cleanUp() throws {
    if try running() { try process(cli, ["server", "stop"]) }
    try FileManager.default.removeItem(at: directory)
  }
}

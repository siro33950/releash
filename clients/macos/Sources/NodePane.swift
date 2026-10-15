import SwiftUI

struct NodePane: View {
  @Bindable var model: AppModel
  let path: String
  let nodeID: String
  let focused: Bool
  @State private var detail: Releash_Client_V1_WorkspaceNodeDetailDto?
  @State private var subscription: UUID?

  var body: some View {
    Group {
      if let detail {
        if case .session(let session) = detail.content.variant, session.hasSessionID {
          AgentSessionPane(model: model, path: path, nodeID: nodeID, sessionID: session.sessionID, canResume: detail.capabilities.canResumeSession, focused: focused).id(session.sessionID)
        } else if case .command(let command) = detail.content.variant {
          CommandPane(model: model, path: path, detail: detail, command: command)
        } else { Text("Nodeの内容がありません").foregroundStyle(.secondary) }
      } else { ProgressView() }
    }.frame(maxWidth: .infinity, maxHeight: .infinity)
    .onAppear {
      if let subscription { model.unsubscribe(subscription) }
      subscription = model.subscribe("node-detail", [path, nodeID]) { payload in
        if case .nodeDetail(let value) = payload.value {
          detail = value.hasValue ? value.value : nil
        }
      }
    }
    .onDisappear { if let subscription { model.unsubscribe(subscription) }; subscription = nil }
  }
}

struct AgentSessionPane: View {
  @Bindable var model: AppModel
  let path: String
  let nodeID: String
  let sessionID: String
  let canResume: Bool
  let focused: Bool
  @State private var session: Releash_Client_V1_AgentSessionItemDto?
  @State private var subscription: UUID?
  @State private var failure: String?
  var body: some View {
    VStack {
      if let failure { Text(failure).foregroundStyle(.red).textSelection(.enabled) }
      switch session?.availability {
      case .open: TerminalPane(session: model.terminal(path: path, id: sessionID, agent: true), focused: focused)
      case .paused:
        Text("Sessionは停止しています")
        if canResume { Button("Resume") { model.resumeNode(path: path, id: nodeID) } }
      case .archived: Text("SessionはArchiveされています")
      case .indeterminate: Text("Sessionを開けるか判定できません")
      default: ProgressView()
      }
    }
    .task {
      subscription = model.subscribe("agent-session", [sessionID]) { payload in
        if case .agentSession(let value) = payload.value {
          session = value.hasValue ? value.value : nil
          if !value.hasValue || value.value.availability != .open { model.releaseAgentTerminal(path: path, id: sessionID) }
        }
      }
      guard let client = model.client else { return }
      var request = Releash_Client_V1_OpenAgentSessionRequest()
      request.agentSessionID = sessionID; request.rows = UIStyle.terminalRows; request.cols = UIStyle.terminalColumns; request.callerRequestID = UUID().uuidString
      do { _ = try await client.openAgentSession(request: request, headers: model.headers).result.get() }
      catch { failure = model.failureMessage(error) }
    }
    .onDisappear {
      if let subscription { model.unsubscribe(subscription) }; subscription = nil
      model.releaseAgentTerminal(path: path, id: sessionID)
    }
  }
}

struct CommandPane: View {
  @Bindable var model: AppModel
  let path: String
  let detail: Releash_Client_V1_WorkspaceNodeDetailDto
  let command: Releash_Client_V1_WorkspaceCommandNodeContentDto
  var attempts: [Releash_Client_V1_WorkspaceNodeAttemptDto] { detail.pastAttempts }
  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: UIStyle.padding) {
        HStack(alignment: .top) {
          Text("COMMAND").font(.headline)
          VStack(alignment: .leading) {
            Text(command.displayCommand).font(.system(.body, design: .monospaced))
            Text(command.hasCwd ? command.cwd : "作業ディレクトリは未記録")
            Text(command.hasShell ? command.shell : "shellは未記録")
            if command.hasResult { Text("Exit code: \(command.result.exitCode) · \(command.result.duration) ms").accessibilityIdentifier("command-result") }
          }.textSelection(.enabled)
          Spacer()
          if detail.capabilities.canRetry { Button("Retry") { model.retryNode(path: path, id: detail.id) } }
          if detail.canAbort { Button("Abort") { if detail.hasExecutionID { model.abortExecution(id: detail.executionID) } } }
        }
        if detail.hasErrorReason { Text(detail.errorReason).foregroundStyle(.red).textSelection(.enabled) }
        Text("STDOUT").font(.headline)
        Text(command.result.stdout).font(.system(.body, design: .monospaced)).textSelection(.enabled).accessibilityIdentifier("command-stdout")
        Text("STDERR").font(.headline)
        Text(command.result.stderr).font(.system(.body, design: .monospaced)).textSelection(.enabled).accessibilityIdentifier("command-stderr")
        if !attempts.isEmpty {
          Text("過去の試行").font(.headline)
          ForEach(attempts, id: \.id) { attempt in
            Button { model.openNode(.command, id: attempt.id, title: attempt.title) } label: {
              Text("\(Date(timeIntervalSince1970: attempt.updatedAt).formatted()) · \(attempt.errorReason)")
            }
          }
        }
      }.padding(UIStyle.padding).frame(maxWidth: .infinity, alignment: .leading)
    }
  }
}

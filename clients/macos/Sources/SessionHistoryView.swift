import SwiftUI

struct SessionHistoryView: View {
  @Bindable var model: AppModel
  let path: String
  @Environment(\.dismiss) private var dismiss
  @State private var page = Releash_Client_V1_AgentSessionHistoryPageDto()
  @State private var count = 20
  @State private var subscription: UUID?
  @State private var busy = false
  @State private var failure: String?
  var archived: [Releash_Client_V1_AgentSessionItemDto] {
    model.repositories.flatMap(\.worktrees.items).first { $0.path == path }?.snapshot
      .archivedSessions.items ?? []
  }
  var workflows: [Releash_Client_V1_WorkspaceWorkflowHistoryItem] {
    model.repositories.flatMap(\.worktrees.items).first { $0.path == path }?.workflowHistory.items
      ?? []
  }
  var body: some View {
    VStack(alignment: .leading) {
      Text("History").font(.headline)
      if let failure { Text(failure).foregroundStyle(.red).textSelection(.enabled) }
      List {
        Section("Archive 済みのSession") {
          ForEach(archived, id: \.id) { session in
            HStack {
              Button("\(session.provider.displayName) · \(session.providerSessionID)") {
                restore(session)
              }
              Spacer()
              if session.operations.canDelete {
                Button("Delete", role: .destructive) { delete(session) }
              }
            }
          }
        }
        Section("Archive 済みのWorkflow") {
          ForEach(workflows, id: \.executionID) { workflow in
            Button(workflow.title) { restoreWorkflow(workflow.executionID) }
          }
        }
        Section("Provider の履歴") {
          ForEach(page.items.items, id: \.historyID) { candidate in
            Button("\(candidate.provider.displayName) · \(candidate.label)") { resume(candidate) }
          }
          if page.hasMore_p {
            Button("さらに読み込む") {
              count += 20
              subscribe()
            }
          }
        }
      }.disabled(busy)
      HStack {
        if busy { ProgressView().controlSize(.small) }
        Spacer()
        Button("閉じる") { dismiss() }.disabled(busy)
      }
    }.padding(UIStyle.padding).frame(width: 520, height: 420)
      .onAppear { subscribe() }
      .onDisappear { if let subscription { model.unsubscribe(subscription) } }
  }
  func subscribe() {
    if let subscription { model.unsubscribe(subscription) }
    subscription = model.subscribe("session-history", [path, String(count)]) { payload in
      if case .sessionHistory(let value) = payload.value { page = value }
    }
  }
  func perform(_ action: @escaping () async throws -> Releash_Client_V1_SessionSelection?) {
    busy = true
    failure = nil
    Task {
      defer { busy = false }
      do {
        if let selection = try await action() {
          model.openNode(.session, id: selection.agentSessionID, title: "Session")
          dismiss()
        }
      } catch { failure = model.failureMessage(error) }
    }
  }
  func restoreWorkflow(_ id: String) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_RestoreWorkspaceWorkflowExecutionRequest()
    request.worktreePath = path
    request.executionID = id
    perform {
      _ = try await client.restoreWorkspaceWorkflowExecution(
        request: request, headers: model.headers
      ).result.get()
      dismiss()
      return nil
    }
  }
  func restore(_ session: Releash_Client_V1_AgentSessionItemDto) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_RestoreAgentSessionRequest()
    request.agentSessionID = session.id
    request.rows = UIStyle.terminalRows
    request.cols = UIStyle.terminalColumns
    request.callerRequestID = UUID().uuidString
    perform {
      try await client.restoreAgentSession(request: request, headers: model.headers).result.get()
    }
  }
  func delete(_ session: Releash_Client_V1_AgentSessionItemDto) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_DeleteAgentSessionRequest()
    request.agentSessionID = session.id
    request.callerRequestID = UUID().uuidString
    perform {
      _ = try await client.deleteAgentSession(request: request, headers: model.headers).result.get()
      return nil
    }
  }
  func resume(_ candidate: Releash_Client_V1_AgentSessionHistoryCandidateDto) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_ResumeAgentSessionHistoryCandidateRequest()
    request.workspaceIdentity = path
    request.worktreePath = path
    request.provider = candidate.provider.wireName
    request.providerSessionID = candidate.providerSessionID
    request.rows = UIStyle.terminalRows
    request.cols = UIStyle.terminalColumns
    request.callerRequestID = UUID().uuidString
    perform {
      try await client.resumeAgentSessionHistoryCandidate(request: request, headers: model.headers)
        .result.get()
    }
  }
}

extension Releash_Client_V1_AgentSessionHistoryCandidateDto {
  var historyID: String { provider.wireName + ":" + providerSessionID }
}

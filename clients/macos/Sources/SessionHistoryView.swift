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
  @State private var archive = Releash_Client_V1_ListArchivedHistoryItem()
  @State private var archiveSubscription: UUID?
  @State private var search = ""
  @State private var section = "すべて"
  @State private var selected: String?
  @State private var deleting: Releash_Client_V1_ArchivedSessionHistoryItem?

  var body: some View {
    VStack(alignment: .leading, spacing: UIStyle.padding) {
      HStack {
        Text("History").font(.headline)
        Text(
          model.repositories.lazy.flatMap { $0.branches.items }.first { $0.worktreePath == path }?
            .name ?? ""
        )
        .foregroundStyle(.secondary)
      }
      if let failure { Text(failure).foregroundStyle(.red).textSelection(.enabled) }
      TextField("名前・最初の指示で検索", text: $search)
        .textFieldStyle(.roundedBorder)
        .accessibilityLabel("名前・最初の指示で検索")
      Picker("区画", selection: $section) {
        ForEach(["すべて", "Archive 済み", "Provider の履歴"], id: \.self) { Text($0).tag($0) }
      }.pickerStyle(.segmented)
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 12) {
          if section != "Provider の履歴" {
            Section {
              ForEach(archive.items, id: \.historyID) { item in
                switch item.variant {
                case .session(let session):
                  sessionRow(session, id: item.historyID)
                case .workflow(let workflow):
                  workflowRow(workflow, id: item.historyID)
                case nil: EmptyView()
                }
              }
            } header: {
              HStack {
                Text("ARCHIVE 済み")
                Text(String(archive.items.count))
              }
            }
          }
          if section != "Archive 済み" {
            Section {
              ForEach(page.items.items, id: \.historyID) { candidate in
                providerRow(candidate)
              }
              if page.hasMore_p {
                Button("さらに読み込む") {
                  count += 20
                  subscribeProvider()
                }
              }
            } header: {
              VStack(alignment: .leading) {
                Text("PROVIDER の履歴")
                Text("このディレクトリで provider が残した Session")
              }
            }
          }
        }
      }.disabled(busy)
      HStack {
        Text("行を選ぶと操作が出ます。ダブルクリックで復元・再開します。")
          .foregroundStyle(.secondary)
        if busy { ProgressView().controlSize(.small) }
        Spacer()
        Button("閉じる") { dismiss() }.disabled(busy)
      }
    }.padding(UIStyle.padding).frame(width: 1024, height: 800)
      .onAppear { subscribe() }
      .onChange(of: search) {
        selected = nil
        subscribe()
      }
      .onChange(of: section) { selected = nil }
      .onDisappear {
        if let subscription { model.unsubscribe(subscription) }
        if let archiveSubscription { model.unsubscribe(archiveSubscription) }
      }
      .alert(
        "「\(deleting?.name ?? "")」を削除しますか？",
        isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } })
      ) {
        Button("キャンセル", role: .cancel) { deleting = nil }
        Button("削除", role: .destructive) {
          if let deleting { delete(deleting) }
          deleting = nil
        }
      } message: {
        Text("削除した Session は History から消え、復元できません。")
      }
  }

  private func sessionRow(_ session: Releash_Client_V1_ArchivedSessionHistoryItem, id: String)
    -> some View
  {
    let action = { restore(session) }
    return
      historyRow(id: id, title: session.name, action: action) {
        Text(
          "Session · \(session.provider.displayName) · \(historyTime(Date(timeIntervalSince1970: session.archivedAt)))に Archive"
        )
      } buttons: {
        Button("復元", action: action)
        if session.canDelete {
          Button("削除", role: .destructive) { deleting = session }
        }
      }
  }

  private func workflowRow(_ workflow: Releash_Client_V1_WorkspaceWorkflowHistoryItem, id: String)
    -> some View
  {
    let action = { restoreWorkflow(workflow.executionID) }
    let status: (color: Color, text: String) =
      workflow.status.value == .completed
      ? (.green, "完了") : (.red, "中止")
    return
      historyRow(
        id: id, title: workflow.title,
        action: action
      ) {
        HStack {
          Text("Workflow ·")
          Circle().fill(status.color)
            .frame(width: 7, height: 7).accessibilityHidden(true)
          Text(status.text)
          Text("· \(historyTime(Date(timeIntervalSince1970: workflow.updatedAt)))")
        }
      } buttons: {
        Button("復元", action: action)
      }
  }

  private func providerRow(_ candidate: Releash_Client_V1_AgentSessionHistoryCandidateDto)
    -> some View
  {
    let action = { resume(candidate) }
    return
      historyRow(
        id: "provider:" + candidate.historyID, title: candidate.label,
        action: action
      ) {
        Text(
          "\(candidate.provider.displayName) · \(historyTime(Date(timeIntervalSince1970: candidate.updatedAt)))"
        )
      } buttons: {
        Button("再開", action: action)
      }
  }

  private func historyRow<Subtitle: View, Buttons: View>(
    id: String, title: String, action: @escaping () -> Void,
    @ViewBuilder subtitle: () -> Subtitle, @ViewBuilder buttons: () -> Buttons
  ) -> some View {
    HStack {
      VStack(alignment: .leading, spacing: 4) {
        Text(title)
        subtitle().font(.caption).foregroundStyle(.secondary)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
      .contentShape(Rectangle())
      .highPriorityGesture(
        TapGesture(count: 2).onEnded { action() }
          .exclusively(before: TapGesture().onEnded { selected = id })
      )
      .accessibilityElement(children: .contain)
      .accessibilityLabel(title)
      .accessibilityAction { selected = id }
      if selected == id { buttons().buttonStyle(.borderless) }
    }
    .padding(.vertical, 4)
    .background(selected == id ? Color.accentColor.opacity(0.12) : Color.clear)
  }

  private var searchArgs: [String] {
    let term = search.trimmingCharacters(in: .whitespacesAndNewlines)
    return term.isEmpty ? [] : ["search=" + term]
  }

  func subscribe() {
    if let archiveSubscription { model.unsubscribe(archiveSubscription) }
    archiveSubscription = model.subscribe("archived-history", [path] + searchArgs) { payload in
      if case .archivedHistory(let value) = payload.value { archive = value }
    }
    subscribeProvider()
  }

  private func subscribeProvider() {
    if let subscription { model.unsubscribe(subscription) }
    subscription = model.subscribe("session-history", [path, String(count)] + searchArgs) {
      payload in
      if case .sessionHistory(let value) = payload.value { page = value }
    }
  }
  func perform(_ action: @escaping () async throws -> Releash_Client_V1_SessionSelection?) {
    guard !busy else { return }
    busy = true
    failure = nil
    Task {
      defer { busy = false }
      do {
        if let selection = try await action() {
          model.openSession(selection, path: path)
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
  func restore(_ session: Releash_Client_V1_ArchivedSessionHistoryItem) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_RestoreAgentSessionRequest()
    request.agentSessionID = session.agentSessionID
    request.rows = UIStyle.terminalRows
    request.cols = UIStyle.terminalColumns
    request.callerRequestID = UUID().uuidString
    perform {
      _ = try await client.restoreAgentSession(request: request, headers: model.headers).result
        .get()
      dismiss()
      return nil
    }
  }
  func delete(_ session: Releash_Client_V1_ArchivedSessionHistoryItem) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_DeleteAgentSessionRequest()
    request.agentSessionID = session.agentSessionID
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

extension Releash_Client_V1_ArchivedHistoryItem {
  fileprivate var historyID: String {
    switch variant {
    case .session(let value): "session:" + value.agentSessionID
    case .workflow(let value): "workflow:" + value.executionID
    case nil: ""
    }
  }
}

func historyTime(_ date: Date, now: Date = Date(), calendar: Calendar = .current) -> String {
  if calendar.isDate(date, inSameDayAs: now) {
    let seconds = max(0, now.timeIntervalSince(date))
    if seconds < 60 { return "たった今" }
    if seconds < 3600 { return "\(Int(seconds / 60)) 分前" }
    return "\(Int(seconds / 3600)) 時間前"
  }
  let format = DateFormatter()
  format.calendar = calendar
  format.timeZone = calendar.timeZone
  format.locale = Locale(identifier: "ja_JP")
  if let yesterday = calendar.date(byAdding: .day, value: -1, to: now),
    calendar.isDate(date, inSameDayAs: yesterday)
  {
    format.dateFormat = "昨日 HH:mm"
  } else if calendar.component(.year, from: date) == calendar.component(.year, from: now) {
    format.dateFormat = "M月d日"
  } else {
    format.dateFormat = "yyyy年M月d日"
  }
  return format.string(from: date)
}

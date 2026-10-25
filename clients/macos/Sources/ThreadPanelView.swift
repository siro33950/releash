import Observation
import SwiftUI
import Textual

struct ThreadPanelView: View {
  var model: AppModel
  let worktree: String
  @State var state = ThreadPanelState()
  @State private var subscription: UUID?
  @State private var sessionSubscriptions: [String: UUID] = [:]
  @State private var addingGeneral = false
  @State private var content = ""

  var sessionNodes: [Releash_Client_V1_WorkspaceNode] {
    Self.sessionNodes(
      model.repositories.flatMap(\.worktrees.items).first { $0.path == worktree }?.snapshot.nodes
        .items ?? [])
  }
  var sessionIDs: [String] { sessionNodes.map(\.sessionID) }
  var openSessions: [Releash_Client_V1_WorkspaceNode] {
    sessionNodes.filter { state.sessionAvailability[$0.sessionID] == .open }
  }

  var body: some View {
    VStack(spacing: 0) {
      VStack(alignment: .leading, spacing: UIStyle.spacing) {
        HStack {
          Text("Thread").font(.headline)
          Spacer(minLength: 0)
        }
        Picker("状態", selection: $state.showResolved) {
          Text("未解決 \(state.threads.filter { !$0.isResolved }.count)").tag(false)
          Text("解決済み \(state.threads.filter(\.isResolved).count)").tag(true)
        }
        .pickerStyle(.segmented)
        .accessibilityIdentifier("thread-filter")
        Menu {
          ForEach(openSessions, id: \.sessionID) { node in
            Button(node.title) { send(to: node.sessionID) }
              .accessibilityIdentifier("thread-session:\(node.sessionID)")
          }
        } label: {
          Text("チェックした \(state.selectedIDs.count) 件を Session に送る")
            .frame(maxWidth: .infinity)
        }
        .disabled(state.selectedIDs.isEmpty || openSessions.isEmpty)
        .accessibilityIdentifier("thread-send")
      }.padding(UIStyle.spacing)
      Divider()
      ScrollView {
        LazyVStack(spacing: 0) {
          ForEach(state.visibleThreads, id: \.id) { thread in
            ThreadPanelRow(
              thread: thread,
              checked: Binding(
                get: { state.checked.contains(thread.id) },
                set: {
                  if $0 { state.checked.insert(thread.id) } else { state.checked.remove(thread.id) }
                }),
              reply: { reply(to: thread.id, content: $0, accepted: $1) },
              resolve: { resolve(thread.id) })
            Divider()
          }
        }
      }
      Divider()
      Button("General に追加") {
        content = ""
        addingGeneral = true
      }
      .frame(maxWidth: .infinity).padding(UIStyle.spacing)
      .accessibilityIdentifier("thread-add-general")
    }
    .onAppear {
      subscription = model.subscribe("review-threads", [worktree]) { payload in
        if case .reviewThreads(let value) = payload.value { state.receive(value.items) }
      }
      subscribeSessions()
    }
    .onChange(of: sessionIDs) { _, _ in subscribeSessions() }
    .onDisappear {
      if let subscription { model.unsubscribe(subscription) }
      subscription = nil
      for id in sessionSubscriptions.values { model.unsubscribe(id) }
      sessionSubscriptions.removeAll()
      state.sessionAvailability.removeAll()
    }
    .sheet(isPresented: $addingGeneral) {
      VStack {
        Text("General に追加").font(.headline)
        TextEditor(text: $content).frame(minWidth: 360, minHeight: 120)
          .accessibilityIdentifier("thread-general-content")
        HStack {
          Button("キャンセル") { addingGeneral = false }
            .accessibilityIdentifier("thread-general-cancel")
          Button("作成") {
            createGeneral(content: content) {
              addingGeneral = false
              state.showResolved = false
            }
          }
          .disabled(content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
          .accessibilityIdentifier("thread-general-create")
        }
      }.padding()
    }
  }

  static func sessionNodes(_ items: [Releash_Client_V1_WorkspaceTreeItem])
    -> [Releash_Client_V1_WorkspaceNode]
  {
    items.flatMap { item in
      let nodes: [Releash_Client_V1_WorkspaceNode]
      if case .node(let node) = item.variant, node.hasSessionID {
        nodes = [node]
      } else {
        nodes = []
      }
      return nodes + sessionNodes(item.children)
    }
  }

  private func subscribeSessions() {
    let ids = Set(sessionIDs)
    for id in Array(sessionSubscriptions.keys) where !ids.contains(id) {
      if let subscription = sessionSubscriptions.removeValue(forKey: id) {
        model.unsubscribe(subscription)
      }
      state.sessionAvailability.removeValue(forKey: id)
    }
    for id in sessionIDs where sessionSubscriptions[id] == nil {
      sessionSubscriptions[id] = model.subscribe("session-tab", [id]) { payload in
        if case .sessionTab(let value) = payload.value {
          state.sessionAvailability[id] = value.session.availability
        }
      }
    }
  }

  func reply(to id: String, content: String, accepted: @escaping () -> Void) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_AppendReviewCommentRequest()
    request.worktreeName = worktree
    request.threadID = id
    request.content = content
    model.perform {
      _ = try await client.appendReviewComment(request: request, headers: model.headers).result
        .get()
      accepted()
    }
  }

  func resolve(_ id: String) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_ResolveReviewThreadRequest()
    request.worktreeName = worktree
    request.threadID = id
    request.outcome = "resolved"
    request.summary = "Threadの面で解決"
    model.perform {
      _ = try await client.resolveReviewThread(request: request, headers: model.headers).result
        .get()
    }
  }

  func createGeneral(content: String, accepted: @escaping () -> Void) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_CreateReviewThreadRequest()
    request.worktreeName = worktree
    request.content = content
    model.perform {
      _ = try await client.createReviewThread(request: request, headers: model.headers).result.get()
      accepted()
    }
  }

  func send(to sessionID: String) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_SendThreadsToSessionRequest()
    request.agentSessionID = sessionID
    request.threadIds.items = state.selectedIDs
    model.perform {
      _ = try await client.sendThreadsToSession(request: request, headers: model.headers).result
        .get()
    }
  }
}

@Observable @MainActor final class ThreadPanelState {
  var threads: [Releash_Client_V1_ReviewThreadDto] = []
  var showResolved = false
  var checked: Set<String> = []
  var sessionAvailability: [String: Releash_Client_V1_AgentSessionAvailability] = [:]
  var visibleThreads: [Releash_Client_V1_ReviewThreadDto] {
    threads.filter { $0.isResolved == showResolved }
  }
  var selectedIDs: [String] { threads.filter { checked.contains($0.id) }.map(\.id) }

  func receive(_ threads: [Releash_Client_V1_ReviewThreadDto]) {
    self.threads = threads
    checked.formIntersection(threads.map(\.id))
  }
}

extension Releash_Client_V1_ReviewThreadDto {
  var isResolved: Bool { state.value == .resolved }
  var locationLabel: String {
    guard target.hasFilePath else { return "General" }
    guard target.hasLineNumber else { return target.filePath }
    let end = target.hasEndLine && target.endLine != target.lineNumber ? "–\(target.endLine)" : ""
    return "\(target.filePath):\(target.lineNumber)\(end)"
  }
  var commentCountLabel: String { "Comment \(comments.items.count)件" }
  var lastCommentLabel: String {
    guard let comment = comments.items.last else { return "" }
    let date = Date(timeIntervalSince1970: comment.createdAt).formatted(
      date: .abbreviated, time: .shortened)
    return "\(comment.author.displayName) · \(date)"
  }
}

private struct ThreadPanelRow: View {
  let thread: Releash_Client_V1_ReviewThreadDto
  @Binding var checked: Bool
  let reply: (String, @escaping () -> Void) -> Void
  let resolve: () -> Void
  @State private var replying = false
  @State private var content = ""

  var body: some View {
    VStack(alignment: .leading, spacing: UIStyle.spacing) {
      HStack(alignment: .top) {
        Toggle("送信するThread", isOn: $checked).toggleStyle(.checkbox).labelsHidden()
          .accessibilityIdentifier("thread-check:\(thread.id)")
        Text(thread.locationLabel).font(.caption).textSelection(.enabled)
        Spacer(minLength: 0)
        Text("\(thread.comments.items.count)").font(.caption)
          .accessibilityLabel(thread.commentCountLabel)
          .accessibilityIdentifier("thread-comments:\(thread.id)")
        if thread.isResolved {
          Label("解決済み", systemImage: "checkmark.circle").font(.caption)
            .accessibilityIdentifier("thread-resolved:\(thread.id)")
        }
      }
      if let first = thread.comments.items.first {
        StructuredText(markdown: first.content)
          .accessibilityIdentifier("thread-body:\(thread.id)")
      }
      Text(thread.lastCommentLabel).font(.caption).foregroundStyle(.secondary)
      if thread.comments.items.count > 1 {
        DisclosureGroup("Commentを表示") {
          ForEach(thread.comments.items.dropFirst(), id: \.id) { comment in
            VStack(alignment: .leading) {
              Text(comment.author.displayName).font(.caption).foregroundStyle(.secondary)
              StructuredText(markdown: comment.content)
            }.padding(.vertical, UIStyle.spacing)
          }
        }.accessibilityIdentifier("thread-history:\(thread.id)")
      }
      if !thread.isResolved {
        HStack {
          Button("返信") { replying = true }
            .accessibilityIdentifier("thread-reply:\(thread.id)")
          Button("解決", action: resolve).disabled(!thread.canResolve)
            .accessibilityIdentifier("thread-resolve:\(thread.id)")
        }
      }
    }
    .padding(UIStyle.spacing)
    .frame(maxWidth: .infinity, alignment: .leading)
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier("thread:\(thread.id)")
    .sheet(isPresented: $replying) {
      VStack {
        Text("返信").font(.headline)
        TextEditor(text: $content).frame(minWidth: 360, minHeight: 120)
          .accessibilityIdentifier("thread-reply-content:\(thread.id)")
        HStack {
          Button("キャンセル") { replying = false }
            .accessibilityIdentifier("thread-reply-cancel:\(thread.id)")
          Button("返信を送る") {
            reply(content) {
              content = ""
              replying = false
            }
          }
          .disabled(content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
          .accessibilityIdentifier("thread-reply-send:\(thread.id)")
        }
      }.padding()
    }
  }
}

import Bonsplit
import SwiftUI

struct PaneOpenMenu: View {
  @Bindable var model: AppModel
  let pane: PaneID
  @State private var launch: LaunchKind?
  enum LaunchKind: String, Identifiable {
    case session, workflow
    var id: Self { self }
  }
  var body: some View {
    Menu {
      Button("Session", systemImage: "sparkle") { launch = .session }
      Button("Workflow", systemImage: "play") { launch = .workflow }
      Divider()
      ForEach([PaneTab.Kind.terminal, .workflow, .file], id: \.self) { kind in
        Button(kind.title, systemImage: kind.icon) { model.panes.open(kind, in: pane) }
      }
    } label: {
      Image(systemName: "plus")
    }
    .menuStyle(.borderlessButton)
    .disabled(!model.layoutLoaded)
    .menuIndicator(.hidden)
    .fixedSize()
    .padding(.horizontal, UIStyle.spacing)
    .help("開く")
    .accessibilityLabel("開く")
    .sheet(item: $launch) { kind in
      if let path = model.selectedWorktree {
        PaneLaunchView(model: model, pane: pane, path: path, kind: kind)
      }
    }
  }
}

struct PaneLaunchView: View {
  @Bindable var model: AppModel
  let pane: PaneID
  let path: String
  let kind: PaneOpenMenu.LaunchKind
  @Environment(\.dismiss) private var dismiss
  @State private var provider = ""
  @State private var workflow = ""
  @State private var requestText = ""
  @State private var starting = false
  @State private var failure: String?

  var body: some View {
    VStack(alignment: .leading, spacing: UIStyle.padding) {
      Text(kind == .session ? "Sessionを開始" : "Workflowを開始").font(.headline)
      Form {
        if kind == .session {
          Picker("Provider", selection: $provider) {
            Text("選択してください").tag("")
            ForEach(model.providers, id: \.value) { item in
              Text(item.displayName).tag(item.wireName)
            }
          }
        } else {
          WorkflowLaunchFields(
            workflows: model.workflows, workflow: $workflow, requestText: $requestText)
        }
      }.disabled(starting)
      if let failure { Text(failure).foregroundStyle(.red).textSelection(.enabled) }
      HStack {
        if starting { ProgressView().controlSize(.small) }
        Spacer()
        Button("キャンセル") { dismiss() }.disabled(starting).keyboardShortcut(.cancelAction)
        Button("開始") {
          starting = true
          failure = nil
          Task {
            failure = await start(provider: provider, workflow: workflow, requestText: requestText)
            if failure == nil { dismiss() } else { starting = false }
          }
        }
        .disabled(starting || (kind == .session ? provider.isEmpty : workflow.isEmpty))
        .keyboardShortcut(.defaultAction)
      }
    }
    .padding(UIStyle.padding)
    .frame(width: UIStyle.createDialogWidth)
    .interactiveDismissDisabled(starting)
  }

  func start(provider: String, workflow: String, requestText: String) async -> String? {
    do {
      if kind == .session {
        try await startSession(provider)
      } else {
        try await startWorkflow(name: workflow, requestText: requestText)
      }
      return nil
    } catch { return model.failureMessage(error) }
  }

  func startSession(_ provider: String) async throws {
    guard let client = model.client else { throw CLIFailure(message: "サーバに接続されていません") }
    var request = Releash_Client_V1_CreateAgentSessionRequest()
    request.provider = provider
    request.workspaceIdentity = path
    request.worktreePath = path
    request.rows = UIStyle.terminalRows
    request.cols = UIStyle.terminalColumns
    request.callerRequestID = UUID().uuidString
    _ = try await client.createAgentSession(request: request, headers: model.headers).result.get()
    model.panes.open(.session, in: pane)
  }

  func startWorkflow(name: String, requestText: String) async throws {
    guard let client = model.client else { throw CLIFailure(message: "サーバに接続されていません") }
    var request = Releash_Client_V1_StartWorkflowRequest()
    request.workflowName = name
    request.worktreePath = path
    request.request = requestText
    _ = try await client.startWorkflow(request: request, headers: model.headers).result.get()
    model.panes.open(.workflow, in: pane)
  }
}

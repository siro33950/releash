import Connect
import SwiftUI

struct GeneralSettingsView: View {
  var model: AppModel
  @State var state = GeneralSettingsState()
  @AppStorage("diffBase") var diffBase = "head"
  @FocusState private var editingCommand: Bool

  var body: some View {
    Form {
      Section {
        Picker(
          "外部エディタ",
          selection: Binding(
            get: { state.editor?.selected ?? "" },
            set: { value in Task { await state.updateEditor(value, model: model) } })
        ) {
          Text("システム既定").tag("")
          ForEach(state.editor?.editors.items ?? [], id: \.path) { editor in
            Text(editor.name).tag(editor.path)
          }
          if let editor = state.editor, !editor.selected.isEmpty,
            !editor.editors.items.contains(where: { $0.path == editor.selected })
          {
            Text(editor.selected).tag(editor.selected)
          }
        }
        .disabled(!model.connected || state.editor == nil || state.saving.contains(.editor))
        .accessibilityIdentifier("settings-external-editor")
        TextField("Terminalの起動コマンド", text: $state.command)
          .focused($editingCommand)
          .onSubmit { saveCommand() }
          .onChange(of: editingCommand) { _, editing in
            if !editing { saveCommand() }
          }
          .disabled(
            !model.connected || state.terminalCommand == nil || state.saving.contains(.command)
          )
          .accessibilityIdentifier("settings-terminal-command")
        Toggle(
          "承認の自動化",
          isOn: Binding(
            get: { state.workflow?.approvalAutoApprove ?? false },
            set: { value in Task { await state.updateApproval(value, model: model) } })
        )
        .disabled(!model.connected || state.workflow == nil || state.saving.contains(.approval))
        .accessibilityIdentifier("settings-approval-automation")
      }
      Section {
        Picker("Diff baseの既定", selection: $diffBase) {
          Text("HEAD").tag("head")
          Text("Branch base").tag("branch-base")
        }
        .accessibilityIdentifier("settings-diff-base")
      }
      if let failure = state.failure {
        Section {
          Text(failure).foregroundStyle(.secondary)
            .accessibilityIdentifier("settings-save-failure")
        }
      }
    }
    .formStyle(.grouped)
    .onAppear { state.subscribe(model) }
    .onDisappear {
      saveCommand()
      state.unsubscribe(model)
    }
  }

  private func saveCommand() {
    Task { await state.updateCommand(model: model) }
  }
}

@Observable @MainActor final class GeneralSettingsState {
  enum Field { case editor, command, approval }
  var editor: Releash_Client_V1_ExternalEditorState?
  var workflow: Releash_Client_V1_WorkflowSection?
  var terminalCommand: String?
  var command = ""
  var failure: String?
  var saving: Set<Field> = []
  @ObservationIgnored private var subscriptions: [UUID] = []

  func subscribe(_ model: AppModel) {
    guard subscriptions.isEmpty else { return }
    subscriptions = [
      model.subscribe("external-editor") { [weak self] payload in
        if case .externalEditor(let value) = payload.value { self?.editor = value }
      },
      model.subscribe("terminal-startup-command") { [weak self] payload in
        if case .terminalStartupCommand(let value) = payload.value {
          self?.terminalCommand = value.value
          self?.command = value.value
        }
      },
      model.subscribe("workflow-config") { [weak self] payload in
        if case .workflowConfig(let value) = payload.value { self?.workflow = value }
      },
    ]
  }

  func unsubscribe(_ model: AppModel) {
    for id in subscriptions { model.unsubscribe(id) }
    subscriptions = []
  }

  func updateEditor(_ value: String, model: AppModel) async {
    await save(.editor, model) { client, headers in
      var request = Releash_Client_V1_UpdateExternalEditorRequest()
      request.editor = value
      _ = try await client.updateExternalEditor(request: request, headers: headers).result.get()
    }
  }

  func updateApproval(_ value: Bool, model: AppModel) async {
    await save(.approval, model) { client, headers in
      var request = Releash_Client_V1_UpdateWorkflowConfigRequest()
      request.workflow.approvalAutoApprove = value
      _ = try await client.updateWorkflowConfig(request: request, headers: headers).result.get()
    }
  }

  func updateCommand(model: AppModel) async {
    guard let terminalCommand, command != terminalCommand else { return }
    let value = command
    await save(.command, model) { client, headers in
      var request = Releash_Client_V1_UpdateTerminalStartupCommandRequest()
      request.command = value
      _ = try await client.updateTerminalStartupCommand(request: request, headers: headers).result
        .get()
    }
  }

  private func save(
    _ field: Field, _ model: AppModel,
    operation: (any Releash_Client_V1_ClientServiceClientInterface, Headers) async throws -> Void
  ) async {
    guard saving.insert(field).inserted else { return }
    failure = nil
    defer {
      saving.remove(field)
      if field == .command { command = terminalCommand ?? "" }
    }
    do {
      guard let client = model.client, model.connected else {
        throw CLIFailure(message: "サーバに接続されていません")
      }
      try await operation(client, model.headers)
    } catch {
      failure = model.failureMessage(error)
    }
  }
}

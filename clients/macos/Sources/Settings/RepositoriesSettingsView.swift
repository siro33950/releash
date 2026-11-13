import Connect
import SwiftUI

struct RepositoriesSettingsView: View {
  var model: AppModel
  @State var state = RepositoriesSettingsState()

  var body: some View {
    VStack(spacing: 0) {
      if let selected = state.selected,
        let repository = model.repositories.first(where: { $0.path == selected.path })
      {
        HStack {
          Button("戻る") { state.selected = nil }
          Text(repository.name).font(.title2)
          Spacer()
        }
        .padding()
        RepositorySettingsView(model: model, state: selected).id(selected.path)
      } else {
        List(model.repositories, id: \.path) { repository in
          Button {
            state.selected = RepositorySettingsState(path: repository.path)
          } label: {
            HStack {
              VStack(alignment: .leading) {
                Text(repository.name).font(.headline)
                Text(repository.path).foregroundStyle(.secondary)
              }
              Spacer()
            }
            .padding(.vertical, 4)
          }
          .buttonStyle(.plain)
          .accessibilityIdentifier("settings-repository-\(repository.path)")
        }
        .accessibilityIdentifier("settings-repositories")
        .overlay {
          if model.repositories.isEmpty {
            ContentUnavailableView { Label("リポジトリを選択してください", image: "folder") }
          }
        }
      }
    }
    .onChange(of: model.repositories.map(\.path)) { _, paths in
      if let selected = state.selected, !paths.contains(selected.path) {
        state.selected = nil
      }
    }
  }

}

@Observable @MainActor final class RepositoriesSettingsState {
  var selected: RepositorySettingsState?
}

struct RepositorySettingsView: View {
  var model: AppModel
  @State var state: RepositorySettingsState
  @State private var confirmingRemoval = false

  init(model: AppModel, state: RepositorySettingsState) {
    self.model = model
    _state = State(initialValue: state)
  }

  var body: some View {
    Form {
      Section("Repository") {
        LabeledContent("パス") { Text(state.path).textSelection(.enabled) }
        Picker(
          selection: Binding(
            get: { state.base?.value ?? "" },
            set: { value in Task { await state.updateBase(value, model: model) } })
        ) {
          Text("自動（mainまたはmaster）").tag("")
          ForEach(state.branches?.items ?? [], id: \.name) { branch in
            Text(branch.name).tag(branch.name)
          }
          if let base = state.base?.value, !base.isEmpty,
            !(state.branches?.items.contains { $0.name == base } ?? false)
          {
            Text(base).tag(base)
          }
        } label: {
          SettingsLabel(
            title: "Base branch",
            description: "ファイルタブで「base と比較」を選んだときに、比べる相手になるブランチです。")
        }
        .disabled(state.base == nil || state.branches == nil || state.saving.contains(.base))
        .accessibilityIdentifier("settings-base-branch")
      }
      Section("Notion の連携") {
        SecureField("API token", text: $state.apiToken)
          .accessibilityIdentifier("settings-notion-token")
        TextField("Database ID", text: $state.databaseID)
          .accessibilityIdentifier("settings-notion-database")
        Button("接続テスト") { Task { await state.validateNotion(model: model) } }
          .accessibilityIdentifier("settings-notion-validate")
        Text(state.validationMessage).foregroundStyle(.secondary)
          .accessibilityIdentifier("settings-notion-status")
        if state.validation?.status.value == .configured {
          Picker("Title", selection: $state.mapping.title) {
            Text("未設定").tag("")
            ForEach(state.validation?.properties.items ?? [], id: \.name) { property in
              Text("\(property.name) (\(property.propertyType))").tag(property.name)
            }
          }
          .accessibilityIdentifier("settings-notion-title")
          SettingsLabel(
            title: "Labels", description: "新しい Worktree のダイアログで、Notion task を絞り込むのに使います。")
          ForEach(state.validation?.properties.items ?? [], id: \.name) { property in
            Toggle(
              "Labels: \(property.name) (\(property.propertyType))",
              isOn: Binding(
                get: { state.mapping.labels.items.contains { $0.name == property.name } },
                set: { selected in
                  state.mapping.labels.items.removeAll { $0.name == property.name }
                  if selected {
                    var label = Releash_Client_V1_LabelPropertyView()
                    label.name = property.name
                    label.propertyType = property.propertyType
                    state.mapping.labels.items.append(label)
                  }
                })
            )
            .accessibilityIdentifier("settings-notion-label-\(property.name)")
          }
          Picker("Branch名", selection: $state.mapping.branchName) {
            Text("未設定").tag("")
            ForEach(state.validation?.properties.items ?? [], id: \.name) { property in
              Text("\(property.name) (\(property.propertyType))").tag(property.name)
            }
          }
          .accessibilityIdentifier("settings-notion-branch")
          TextField("Branch名のprefix", text: $state.mapping.branchPrefix)
            .accessibilityIdentifier("settings-notion-prefix")
          Button("Notion設定を保存") { Task { await state.saveNotion(model: model) } }
            .accessibilityIdentifier("settings-notion-save")
        } else if let saved = state.notion, saved.hasValue {
          LabeledContent("Title", value: saved.value.propertyMapping.title)
          LabeledContent(
            "Labels",
            value: saved.value.propertyMapping.labels.items.map(\.name).joined(separator: ", "))
          LabeledContent("Branch名", value: saved.value.propertyMapping.branchName)
          LabeledContent("Branch名のprefix", value: saved.value.propertyMapping.branchPrefix)
        }
        if state.notion?.hasValue == true {
          Button("Notion設定を削除", role: .destructive) {
            Task { await state.deleteNotion(model: model) }
          }
          .accessibilityIdentifier("settings-notion-delete")
        }
      }
      .disabled(
        state.notion == nil || state.saving.contains(.notion) || state.saving.contains(.validation))
      Section("一覧からの削除") {
        Button("Repositoryを一覧から削除", role: .destructive) { confirmingRemoval = true }
          .accessibilityIdentifier("settings-remove-repository")
          .alert("Repositoryを一覧から削除しますか？", isPresented: $confirmingRemoval) {
            Button("削除", role: .destructive) {
              Task { await state.removeRepository(model: model) }
            }
            Button("キャンセル", role: .cancel) {}
          } message: {
            Text("ディスク上のRepositoryは削除されません。")
          }
      }
      if let failure = state.failure {
        Section {
          Text(failure).foregroundStyle(.secondary)
            .accessibilityIdentifier("settings-repository-failure")
        }
      }
    }
    .formStyle(.grouped)
    .disabled(!model.connected || state.saving.contains(.repository))
    .onAppear { state.subscribe(model) }
    .onDisappear { state.unsubscribe(model) }
  }

}

@Observable @MainActor final class RepositorySettingsState: SettingsSaving {
  enum Field { case repository, base, notion, validation }
  let path: String
  var base: Releash_Client_V1_Nullablestring?
  var branches: Releash_Client_V1_ListBranchDto?
  var notion: Releash_Client_V1_NullableNotionRepoConfigView?
  var apiToken = "" { didSet { if apiToken != oldValue { validation = nil } } }
  var databaseID = "" { didSet { if databaseID != oldValue { validation = nil } } }
  var mapping = Releash_Client_V1_PropertyMappingView()
  var validation: Releash_Client_V1_NotionValidationResultView?
  var failure: String?
  var saving: Set<Field> = []
  @ObservationIgnored private var subscriptions: [UUID] = []

  init(path: String) { self.path = path }

  var validationMessage: String {
    switch validation?.status.value {
    case .configured: return "接続に成功しました"
    case .invalidToken: return "API tokenが不正です"
    case .invalidDatabase: return "Database IDが不正です"
    case .networkError: return "ネットワークに接続できません"
    case .notConfigured: return "未設定です"
    case .UNRECOGNIZED: return "接続テストの結果を確認できません"
    case nil: return notion?.hasValue == true ? "保存済みです。接続テストでpropertyを選択できます" : "未設定です"
    }
  }

  func subscribe(_ model: AppModel) {
    guard subscriptions.isEmpty else { return }
    subscriptions = [
      model.subscribe("releash-base", [path]) { [weak self] payload in
        if case .releashBase(let value) = payload.value { self?.base = value }
      },
      model.subscribe("branches", [path]) { [weak self] payload in
        if case .branches(let value) = payload.value { self?.branches = value }
      },
      model.subscribe("notion-config", [path]) { [weak self] payload in
        if case .notionConfig(let value) = payload.value {
          guard let self else { return }
          let restore =
            notion != value && apiToken == (notion?.value.apiToken ?? "")
            && databaseID == (notion?.value.databaseID ?? "")
            && mapping == (notion?.value.propertyMapping ?? .init())
          notion = value
          if restore { restoreNotion() }
        }
      },
    ]
  }

  func unsubscribe(_ model: AppModel) {
    for id in subscriptions { model.unsubscribe(id) }
    subscriptions = []
  }

  func removeRepository(model: AppModel) async {
    await save(.repository, model) { client, headers in
      var request = Releash_Client_V1_RemoveRepoPathRequest()
      request.path = self.path
      _ = try await client.removeRepoPath(request: request, headers: headers).result.get()
    }
  }

  func updateBase(_ value: String, model: AppModel) async {
    await save(.base, model) { client, headers in
      var request = Releash_Client_V1_SetReleashBaseRequest()
      request.repoPath = self.path
      if !value.isEmpty { request.base = value }
      _ = try await client.setReleashBase(request: request, headers: headers).result.get()
    }
  }

  func validateNotion(model: AppModel) async {
    guard !saving.contains(.notion) else { return }
    validation = nil
    var request = Releash_Client_V1_ValidateNotionConfigRequest()
    request.apiToken = apiToken
    request.databaseID = databaseID
    await save(.validation, model) { client, headers in
      let value = try await client.validateNotionConfig(request: request, headers: headers).result
        .get()
      if request.apiToken == self.apiToken && request.databaseID == self.databaseID {
        self.validation = value
      }
    }
  }

  func saveNotion(model: AppModel) async {
    guard validation?.status.value == .configured, !saving.contains(.validation) else { return }
    var request = Releash_Client_V1_SaveNotionConfigRequest()
    request.repoPath = path
    request.apiToken = apiToken
    request.databaseID = databaseID
    request.propertyMapping = mapping
    await save(.notion, model) { client, headers in
      _ = try await client.saveNotionConfig(request: request, headers: headers).result.get()
    }
  }

  func deleteNotion(model: AppModel) async {
    guard !saving.contains(.validation) else { return }
    await save(.notion, model) { client, headers in
      var request = Releash_Client_V1_DeleteNotionConfigRequest()
      request.repoPath = self.path
      _ = try await client.deleteNotionConfig(request: request, headers: headers).result.get()
    }
  }

  private func restoreNotion() {
    apiToken = notion?.value.apiToken ?? ""
    databaseID = notion?.value.databaseID ?? ""
    mapping = notion?.value.propertyMapping ?? .init()
    validation = nil
  }

  private func save(
    _ field: Field, _ model: AppModel,
    operation: (any Releash_Client_V1_ClientServiceClientInterface, Headers) async throws -> Void
  ) async {
    await saveSetting(
      field, model,
      cleanup: {
        if field == .notion { self.restoreNotion() }
      }, operation: operation)
  }
}

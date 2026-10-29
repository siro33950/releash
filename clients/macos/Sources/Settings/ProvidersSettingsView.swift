import Connect
import SwiftUI

struct ProvidersSettingsView: View {
  var model: AppModel
  @State var state = ProvidersSettingsState()
  @FocusState private var editingProvider: String?

  var body: some View {
    Form {
      if let snapshot = state.snapshot {
        ForEach(snapshot.providers.items, id: \.provider) { provider in
          Section(provider.displayName) {
            LabeledContent("可用性", value: provider.available ? "利用可能" : "利用不可")
            LabeledContent("既定の実行ファイル", value: provider.defaultExecutable)
            LabeledContent("有効な実行ファイル", value: provider.effectiveExecutable)
            if provider.available {
              LabeledContent("解決された実行ファイル", value: provider.resolvedExecutable)
            } else {
              LabeledContent("利用できない理由", value: provider.unavailableReason)
            }
            TextField(
              "実行ファイルの上書き",
              text: Binding(
                get: { state.executables[provider.provider] ?? provider.configuredExecutable },
                set: { state.executables[provider.provider] = $0 })
            )
            .focused($editingProvider, equals: provider.provider)
            .onSubmit { saveExecutable(provider.provider) }
            .accessibilityIdentifier("settings-provider-executable-\(provider.provider)")
            Button("リセット") {
              state.executables[provider.provider] = provider.configuredExecutable
              Task { await state.resetExecutable(provider.provider, model: model) }
            }
            .disabled(!provider.hasConfiguredExecutable)
            .accessibilityIdentifier("settings-provider-reset-\(provider.provider)")
          }
          .disabled(
            !model.connected || state.saving.contains(.executable(provider.provider))
              || state.saving.contains(.refresh))
        }
      } else {
        ProgressView()
      }
      Section {
        Button("可用性を再確認") {
          Task { await state.refresh(model: model) }
        }
        .disabled(!model.connected || state.snapshot == nil || !state.saving.isEmpty)
        .accessibilityIdentifier("settings-provider-refresh")
      }
      if let failure = state.failure {
        Section {
          Text(failure).foregroundStyle(.secondary)
            .accessibilityIdentifier("settings-provider-failure")
        }
      }
    }
    .formStyle(.grouped)
    .onAppear { state.subscribe(model) }
    .onChange(of: editingProvider) { previous, _ in
      if let previous { saveExecutable(previous) }
    }
    .onDisappear {
      if let editingProvider { saveExecutable(editingProvider) }
      state.unsubscribe(model)
    }
  }

  private func saveExecutable(_ provider: String) {
    Task { await state.updateExecutable(provider, model: model) }
  }
}

@Observable @MainActor final class ProvidersSettingsState {
  enum Field: Hashable {
    case executable(String)
    case refresh
  }
  var snapshot: Releash_Client_V1_ProviderAvailabilitySnapshotResponse?
  var executables: [String: String] = [:]
  var failure: String?
  var saving: Set<Field> = []
  @ObservationIgnored private var subscription: UUID?

  func subscribe(_ model: AppModel) {
    guard subscription == nil else { return }
    subscription = model.subscribe("provider-availability") { [weak self] payload in
      if case .providerAvailability(let value) = payload.value {
        guard let self else { return }
        for provider in value.providers.items {
          let previous = snapshot?.providers.items.first { $0.provider == provider.provider }
          if executables[provider.provider] == nil
            || executables[provider.provider] == previous?.configuredExecutable
          {
            executables[provider.provider] = provider.configuredExecutable
          }
        }
        snapshot = value
      }
    }
  }

  func unsubscribe(_ model: AppModel) {
    if let subscription { model.unsubscribe(subscription) }
    subscription = nil
  }

  func updateExecutable(_ provider: String, model: AppModel) async {
    guard let saved = snapshot?.providers.items.first(where: { $0.provider == provider }),
      let executable = executables[provider], executable != saved.configuredExecutable
    else { return }
    await save(.executable(provider), model) { client, headers in
      var request = Releash_Client_V1_UpdateProviderExecutableRequest()
      request.provider = provider
      request.executable = executable
      _ = try await client.updateProviderExecutable(request: request, headers: headers).result.get()
    }
  }

  func resetExecutable(_ provider: String, model: AppModel) async {
    await save(.executable(provider), model) { client, headers in
      var request = Releash_Client_V1_ResetProviderExecutableRequest()
      request.provider = provider
      _ = try await client.resetProviderExecutable(request: request, headers: headers).result.get()
    }
  }

  func refresh(model: AppModel) async {
    await save(.refresh, model) { client, headers in
      _ = try await client.refreshProviderAvailability(request: .init(), headers: headers).result
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
      if case .executable(let provider) = field {
        executables[provider] =
          snapshot?.providers.items.first { $0.provider == provider }?
          .configuredExecutable
      }
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

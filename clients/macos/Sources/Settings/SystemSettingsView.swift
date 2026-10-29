import Connect
import ServiceManagement
import SwiftUI

struct SystemSettingsView: View {
  var model: AppModel
  @State var state = SystemSettingsState()
  var openLoginItems: () -> Void = { SMAppService.openSystemSettingsLoginItems() }
  @Environment(\.scenePhase) private var scenePhase

  var body: some View {
    Form {
      Section("ログイン項目") {
        Toggle(
          "ログイン時に起動",
          isOn: Binding(
            get: { state.settings?.autoLaunch ?? false },
            set: { value in Task { await state.updateAutoLaunch(value, model: model) } })
        )
        .disabled(!model.connected || state.settings == nil || state.saving.contains(.login))
        .accessibilityIdentifier("settings-auto-launch")
        Toggle(
          "最小化状態で起動",
          isOn: Binding(
            get: { state.settings?.startMinimized ?? false },
            set: { value in Task { await state.updateStartMinimized(value, model: model) } })
        )
        .disabled(
          !model.connected || state.settings?.autoLaunch != true
            || state.saving.contains(.minimized)
        )
        .accessibilityIdentifier("settings-start-minimized")
        if state.requiresApproval {
          Text("ログイン項目は承認待ちです").foregroundStyle(.secondary)
          Button("システム設定のログイン項目を開く", action: openLoginItems)
            .accessibilityIdentifier("settings-open-login-items")
        }
        if let registration = state.loginRegistration,
          registration.status == .translocated || registration.status == .readOnly
        {
          Text(registration.reason).foregroundStyle(.secondary)
            .accessibilityIdentifier("settings-login-registration-reason")
        }
      }
      Section("CLIの設置") {
        Text("/usr/local/bin/releash")
          .textSelection(.enabled)
        if let installation = state.cliInstallation {
          if installation.status == .allowed {
            Text("CLIを設置できます").foregroundStyle(.secondary)
          } else {
            Text(installation.reason).foregroundStyle(.secondary)
              .accessibilityIdentifier("settings-cli-installation-reason")
          }
        }
        Button("CLIを設置") { Task { await state.installCli(model: model) } }
          .disabled(
            !model.connected || state.cliInstallation?.status != .allowed
              || state.saving.contains(.cli)
          )
          .accessibilityIdentifier("settings-install-cli")
        if let result = state.cliResult {
          switch result.status {
          case .installed: Text("CLIを設置しました")
          case .alreadyInstalled: Text("CLIは設置済みです")
          default: Text("CLIの設置結果を確認できません")
          }
          Text(result.path).textSelection(.enabled)
        }
      }
      Section("Telemetry") {
        Toggle(
          "Performance telemetry",
          isOn: Binding(
            get: { state.settings?.performanceTelemetry ?? false },
            set: { value in Task { await state.updatePerformanceTelemetry(value, model: model) } })
        )
        .disabled(!model.connected || state.settings == nil || state.saving.contains(.performance))
        .accessibilityIdentifier("settings-performance-telemetry")
        Toggle(
          "Crash reporting",
          isOn: Binding(
            get: { state.settings?.crashReporting ?? false },
            set: { value in Task { await state.updateCrashReporting(value, model: model) } })
        )
        .disabled(!model.connected || state.settings == nil || state.saving.contains(.crash))
        .accessibilityIdentifier("settings-crash-reporting")
      }
      if let failure = state.failure {
        Section {
          Text(failure).foregroundStyle(.secondary)
            .accessibilityIdentifier("settings-system-failure")
        }
      }
    }
    .formStyle(.grouped)
    .onAppear {
      state.subscribe(model)
      state.refreshLoginItemStatus()
    }
    .onDisappear { state.unsubscribe(model) }
    .onChange(of: scenePhase) { _, phase in
      if phase == .active { state.refreshLoginItemStatus() }
    }
    .task(id: model.connected) {
      if model.connected {
        await state.checkLoginRegistration(Bundle.main.executablePath, model: model)
      }
    }
  }
}

@Observable @MainActor final class SystemSettingsState {
  enum Field { case login, minimized, performance, crash, cli, registration }
  var settings: Releash_Client_V1_DesktopSettings?
  var cliInstallation: Releash_Client_V1_CliInstallationResult?
  var cliResult: Releash_Client_V1_InstallCliResponse?
  var loginRegistration: Releash_Client_V1_LoginRegistrationResult?
  var requiresApproval = false
  var failure: String?
  var saving: Set<Field> = []
  @ObservationIgnored var loginItemStatus: () -> SMAppService.Status = {
    SMAppService.mainApp.status
  }
  @ObservationIgnored private var subscriptions: [UUID] = []

  func refreshLoginItemStatus() {
    requiresApproval = loginItemStatus() == .requiresApproval
  }

  func subscribe(_ model: AppModel) {
    guard subscriptions.isEmpty else { return }
    subscriptions = [
      model.subscribe("desktop-settings") { [weak self] payload in
        if case .desktopSettings(let value) = payload.value { self?.settings = value }
      },
      model.subscribe("daemon-info") { [weak self] payload in
        if case .daemonInfo(let value) = payload.value {
          self?.cliInstallation = value.cliInstallation
        }
      },
    ]
  }

  func unsubscribe(_ model: AppModel) {
    for id in subscriptions { model.unsubscribe(id) }
    subscriptions = []
  }

  func updateAutoLaunch(_ value: Bool, model: AppModel) async {
    await save(.login, model) { client, headers in
      var request = Releash_Client_V1_UpdateLoginItemPreferenceRequest()
      request.requested = value
      _ = try await client.updateLoginItemPreference(request: request, headers: headers).result
        .get()
    }
  }

  func updateStartMinimized(_ value: Bool, model: AppModel) async {
    await save(.minimized, model) { client, headers in
      var request = Releash_Client_V1_UpdateAppSettingsRequest()
      request.app.startMinimized = value
      _ = try await client.updateAppSettings(request: request, headers: headers).result.get()
    }
  }

  func updatePerformanceTelemetry(_ value: Bool, model: AppModel) async {
    await save(.performance, model) { client, headers in
      var request = Releash_Client_V1_UpdatePerformanceTelemetryRequest()
      request.enabled = value
      _ = try await client.updatePerformanceTelemetry(request: request, headers: headers).result
        .get()
    }
  }

  func updateCrashReporting(_ value: Bool, model: AppModel) async {
    await save(.crash, model) { client, headers in
      var request = Releash_Client_V1_UpdateCrashReportingRequest()
      request.enabled = value
      _ = try await client.updateCrashReporting(request: request, headers: headers).result.get()
    }
  }

  func installCli(model: AppModel) async {
    cliResult = nil
    await save(.cli, model) { client, headers in
      self.cliResult = try await client.installCli(request: .init(), headers: headers).result.get()
    }
  }

  func checkLoginRegistration(_ executablePath: String?, model: AppModel) async {
    await save(.registration, model) { client, headers in
      guard let executablePath else {
        throw CLIFailure(message: "アプリの実行ファイルのパスを取得できません")
      }
      var request = Releash_Client_V1_CheckLoginRegistrationRequest()
      request.executablePath = executablePath
      self.loginRegistration = try await client.checkLoginRegistration(
        request: request, headers: headers
      ).result.get()
    }
  }

  private func save(
    _ field: Field, _ model: AppModel,
    operation: (any Releash_Client_V1_ClientServiceClientInterface, Headers) async throws -> Void
  ) async {
    guard saving.insert(field).inserted else { return }
    failure = nil
    defer { saving.remove(field) }
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
